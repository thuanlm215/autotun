//! Desktop frontend (egui). Compiled only with `--features gui`.

use std::time::Duration;

use anyhow::Result;
use eframe::egui::{self, Color32, RichText};

use crate::{
    cli::Cli,
    engine::{self, Engine},
    ports::Direction,
};

pub fn run(cli: &Cli) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("autotun")
            .with_inner_size([960.0, 560.0])
            .with_min_inner_size([720.0, 400.0]),
        ..Default::default()
    };
    let app = GuiApp::from_cli(cli);
    eframe::run_native("autotun", options, Box::new(|_cc| Ok(Box::new(app))))
        .map_err(|error| anyhow::anyhow!("GUI failed: {error}"))
}

struct GuiApp {
    destination: String,
    reverse_text: String,
    ssh_args_text: String,
    interval_text: String,
    include_loopback: bool,
    auto_forward: bool,
    connect_error: Option<String>,
    session: Option<SessionUi>,
}

struct SessionUi {
    engine: Engine,
    filter: String,
    form: Option<FormUi>,
    form_error: Option<String>,
    clip_status: Option<String>,
}

struct FormUi {
    title: String,
    direction: Direction,
    source: String,
    requested: String,
    label: String,
    edit_index: Option<usize>,
    was_enabled: bool,
}

impl GuiApp {
    fn from_cli(cli: &Cli) -> Self {
        let reverse_text = cli
            .reverse_ports
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let ssh_args_text = cli.ssh_args.join(" ");
        let mut app = Self {
            destination: cli.destination.clone().unwrap_or_default(),
            reverse_text,
            ssh_args_text,
            interval_text: cli.interval.to_string(),
            include_loopback: cli.include_loopback,
            auto_forward: !cli.no_auto_forward,
            connect_error: None,
            session: None,
        };
        if cli.destination.is_some() {
            app.try_connect();
        }
        app
    }

    fn try_connect(&mut self) {
        self.connect_error = None;
        let destination = self.destination.trim().to_owned();
        if destination.is_empty() {
            self.connect_error = Some("Destination is required (user@host or SSH alias).".into());
            return;
        }
        let interval = match self.interval_text.trim().parse::<u64>() {
            Ok(value) if value >= 1 => value,
            _ => {
                self.connect_error = Some("Scan interval must be a number of seconds ≥ 1.".into());
                return;
            }
        };
        let reverse_ports = match parse_port_list(&self.reverse_text) {
            Ok(ports) => ports,
            Err(error) => {
                self.connect_error = Some(error);
                return;
            }
        };
        let ssh_args = parse_ssh_args(&self.ssh_args_text);
        match Engine::connect(
            destination,
            ssh_args,
            &reverse_ports,
            self.include_loopback,
            self.auto_forward,
            interval,
        ) {
            Ok(engine) => {
                self.session = Some(SessionUi {
                    engine,
                    filter: String::new(),
                    form: None,
                    form_error: None,
                    clip_status: None,
                });
            }
            Err(error) => self.connect_error = Some(format!("{error:#}")),
        }
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.session.is_some() {
                self.session_ui(ui);
            } else {
                self.connect_ui(ui);
            }
        });
    }
}

impl GuiApp {
    fn connect_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.heading("autotun");
            ui.label("Connect over SSH and manage port forwards.");
        });
        ui.add_space(16.0);

        egui::Grid::new("connect")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Destination");
                ui.add(
                    egui::TextEdit::singleline(&mut self.destination)
                        .desired_width(360.0)
                        .hint_text("user@host or SSH alias"),
                );
                ui.end_row();

                ui.label("Reverse ports");
                ui.add(
                    egui::TextEdit::singleline(&mut self.reverse_text)
                        .desired_width(360.0)
                        .hint_text("optional, e.g. 3000, 8080"),
                );
                ui.end_row();

                ui.label("Extra SSH args");
                ui.add(
                    egui::TextEdit::singleline(&mut self.ssh_args_text)
                        .desired_width(360.0)
                        .hint_text("optional, e.g. -J bastion"),
                );
                ui.end_row();

                ui.label("Scan interval");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.interval_text).desired_width(60.0));
                    ui.label("seconds");
                });
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.checkbox(
            &mut self.auto_forward,
            "Auto-forward discovered remote ports",
        );
        ui.checkbox(
            &mut self.include_loopback,
            "Include remote loopback listeners",
        );
        ui.add_space(12.0);
        if ui
            .add_sized([160.0, 28.0], egui::Button::new("Connect"))
            .clicked()
        {
            self.try_connect();
        }
        if let Some(error) = &self.connect_error {
            ui.add_space(8.0);
            ui.colored_label(Color32::from_rgb(220, 80, 80), error);
        }
    }

    fn session_ui(&mut self, ui: &mut egui::Ui) {
        let mut disconnect = false;
        {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            session.engine.poll();
            ui.horizontal(|ui| {
                ui.heading("autotun");
                ui.label(RichText::new(session.engine.destination()).strong());
                if session.engine.connected() {
                    ui.colored_label(Color32::from_rgb(80, 200, 120), "● connected");
                } else {
                    ui.colored_label(Color32::from_rgb(230, 180, 60), "○ reconnecting");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Disconnect").clicked() {
                        disconnect = true;
                    }
                    if ui.button("Rescan").clicked() {
                        session.engine.rescan();
                    }
                    if ui.button("Forward image").clicked() {
                        match session.engine.push_clipboard_image() {
                            Ok(path) => {
                                session.clip_status =
                                    Some(format!("{path}  (copied — paste in the AI CLI)"));
                            }
                            Err(error) => {
                                session.clip_status = Some(format!("{error:#}"));
                            }
                        }
                    }
                    if ui.button("Reverse").clicked() {
                        session.form = Some(FormUi::new(Direction::Reverse));
                        session.form_error = None;
                    }
                    if ui.button("Forward").clicked() {
                        session.form = Some(FormUi::new(Direction::Local));
                        session.form_error = None;
                    }
                });
            });
            if disconnect {
                session.engine.shutdown();
            }
        }
        if disconnect {
            self.session = None;
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.add(
                egui::TextEdit::singleline(&mut session.filter)
                    .desired_width(240.0)
                    .hint_text("label or port"),
            );
        });
        if let Some(status) = &session.clip_status {
            ui.colored_label(Color32::from_rgb(80, 180, 220), status);
        }
        ui.add_space(6.0);

        let mut save_form = false;
        let mut cancel_form = false;
        if let Some(form) = session.form.as_mut() {
            ui.group(|ui| {
                ui.label(RichText::new(&form.title).strong());
                let (source_label, requested_label) = match form.direction {
                    Direction::Local => ("Remote port", "Local port (optional)"),
                    Direction::Reverse => ("Local port", "Remote port (optional)"),
                };
                ui.horizontal(|ui| {
                    ui.label(source_label);
                    ui.add(egui::TextEdit::singleline(&mut form.source).desired_width(80.0));
                    ui.label(requested_label);
                    ui.add(
                        egui::TextEdit::singleline(&mut form.requested)
                            .desired_width(80.0)
                            .hint_text("same"),
                    );
                    ui.label("Label");
                    ui.add(egui::TextEdit::singleline(&mut form.label).desired_width(140.0));
                    save_form = ui.button("Save").clicked();
                    cancel_form = ui.button("Cancel").clicked();
                });
            });
            if let Some(error) = &session.form_error {
                ui.colored_label(Color32::from_rgb(220, 80, 80), error);
            }
            ui.add_space(6.0);
        }
        if cancel_form {
            session.form = None;
            session.form_error = None;
        } else if save_form && let Some(form) = session.form.take() {
            match engine::tunnel_from_form(
                form.direction,
                &form.source,
                &form.requested,
                &form.label,
            ) {
                Ok(tunnel) => {
                    if let Some(index) = form.edit_index {
                        session.engine.edit(index, tunnel, form.was_enabled);
                    } else {
                        session.engine.add(tunnel);
                    }
                    session.form_error = None;
                }
                Err(error) => {
                    session.form_error = Some(format!("{error:#}"));
                    session.form = Some(form);
                }
            }
        }

        let filter = session.filter.to_lowercase();
        let visible: Vec<usize> = session
            .engine
            .tunnels()
            .iter()
            .enumerate()
            .filter(|(_, tunnel)| {
                if filter.is_empty() {
                    return true;
                }
                tunnel.label.to_lowercase().contains(&filter)
                    || tunnel.source_port.to_string().contains(&filter)
                    || tunnel
                        .bind_port
                        .map(|port| port.to_string().contains(&filter))
                        .unwrap_or(false)
            })
            .map(|(index, _)| index)
            .collect();

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("tunnels")
                .striped(true)
                .num_columns(7)
                .min_col_width(16.0)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("Direction");
                    ui.strong("Label");
                    ui.strong("Remote");
                    ui.strong("Local");
                    ui.strong("URL");
                    ui.strong("Status");
                    ui.strong("");
                    ui.end_row();

                    let mut action = None::<RowAction>;
                    for index in visible {
                        let tunnel = &session.engine.tunnels()[index];
                        let direction = if tunnel.direction == Direction::Local {
                            "Forward"
                        } else {
                            "Reverse"
                        };
                        let (remote, local) = match tunnel.direction {
                            Direction::Local => (
                                tunnel.source_port.to_string(),
                                tunnel
                                    .bind_port
                                    .map(|port| port.to_string())
                                    .unwrap_or_else(|| "auto".into()),
                            ),
                            Direction::Reverse => (
                                tunnel
                                    .bind_port
                                    .map(|port| port.to_string())
                                    .unwrap_or_else(|| tunnel.requested_port.to_string()),
                                tunnel.source_port.to_string(),
                            ),
                        };
                        let status = tunnel.error.clone().unwrap_or_else(|| {
                            if tunnel.enabled {
                                "ON".into()
                            } else if tunnel.manual_off {
                                "MANUAL OFF".into()
                            } else if !tunnel.present {
                                "TARGET DOWN".into()
                            } else {
                                "off".into()
                            }
                        });
                        let url = engine::tunnel_url(tunnel);
                        ui.label(direction);
                        ui.label(if tunnel.label.is_empty() {
                            "—"
                        } else {
                            &tunnel.label
                        });
                        ui.label(remote);
                        ui.label(local);
                        if url == "—" {
                            ui.label("—");
                        } else if ui.link(&url).clicked() {
                            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                        }
                        let status_color = if tunnel.enabled {
                            Color32::from_rgb(80, 200, 120)
                        } else if tunnel.error.is_some() {
                            Color32::from_rgb(220, 80, 80)
                        } else {
                            ui.visuals().text_color()
                        };
                        ui.colored_label(status_color, status);
                        ui.horizontal(|ui| {
                            let toggle_label = if tunnel.enabled { "Off" } else { "On" };
                            if ui.small_button(toggle_label).clicked() {
                                action = Some(RowAction::Toggle(index));
                            }
                            if ui.small_button("Edit").clicked() {
                                action = Some(RowAction::Edit(index));
                            }
                            if ui.small_button("Remove").clicked() {
                                action = Some(RowAction::Delete(index));
                            }
                        });
                        ui.end_row();
                    }
                    match action {
                        Some(RowAction::Toggle(index)) => session.engine.toggle(index),
                        Some(RowAction::Delete(index)) => session.engine.delete(index),
                        Some(RowAction::Edit(index)) => {
                            let tunnel = &session.engine.tunnels()[index];
                            session.form = Some(FormUi::edit(tunnel, index));
                            session.form_error = None;
                        }
                        None => {}
                    }
                });
        });
    }
}

enum RowAction {
    Toggle(usize),
    Edit(usize),
    Delete(usize),
}

impl FormUi {
    fn new(direction: Direction) -> Self {
        Self {
            title: match direction {
                Direction::Local => "Add forward (remote → local)".into(),
                Direction::Reverse => "Add reverse (local → remote)".into(),
            },
            direction,
            source: String::new(),
            requested: String::new(),
            label: String::new(),
            edit_index: None,
            was_enabled: false,
        }
    }

    fn edit(tunnel: &crate::ports::Tunnel, index: usize) -> Self {
        Self {
            title: match tunnel.direction {
                Direction::Local => "Edit forward (remote → local)".into(),
                Direction::Reverse => "Edit reverse (local → remote)".into(),
            },
            direction: tunnel.direction,
            source: tunnel.source_port.to_string(),
            requested: tunnel.requested_port.to_string(),
            label: tunnel.label.clone(),
            edit_index: Some(index),
            was_enabled: tunnel.enabled,
        }
    }
}

fn parse_port_list(text: &str) -> std::result::Result<Vec<u16>, String> {
    let mut ports = Vec::new();
    for token in text.split(|c: char| c == ',' || c.is_whitespace()) {
        if token.is_empty() {
            continue;
        }
        let port = token
            .parse::<u16>()
            .map_err(|_| format!("Invalid reverse port: {token}"))?;
        if port == 0 {
            return Err("Reverse ports must be between 1 and 65535.".into());
        }
        ports.push(port);
    }
    Ok(ports)
}

fn parse_ssh_args(text: &str) -> Vec<String> {
    match shell_words::split(text) {
        Ok(args) => args,
        Err(_) => text.split_whitespace().map(str::to_owned).collect(),
    }
}
