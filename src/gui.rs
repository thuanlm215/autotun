//! Desktop frontend (egui). Compiled only with `--features gui`.

use std::time::Duration;

use anyhow::Result;
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontFamily, FontId, Layout, Margin, Rect, RichText, Stroke,
    TextStyle, Vec2,
};

use crate::{
    cli::Cli,
    engine::{self, Engine},
    ports::Direction,
};

const ACCENT: Color32 = Color32::from_rgb(56, 148, 156);
const ON_COLOR: Color32 = Color32::from_rgb(86, 196, 130);
const WARN_COLOR: Color32 = Color32::from_rgb(230, 176, 80);
const ERR_COLOR: Color32 = Color32::from_rgb(232, 110, 110);
const MUTED: Color32 = Color32::from_rgb(154, 160, 170);
const CLIP_COLOR: Color32 = Color32::from_rgb(130, 196, 220);
const CARD_FILL: Color32 = Color32::from_rgb(30, 32, 38);
const CARD_STROKE: Color32 = Color32::from_rgb(52, 56, 64);
const TABLE_FILL: Color32 = Color32::from_rgb(26, 28, 33);
const ROW_HOVER: Color32 = Color32::from_rgb(40, 44, 52);
const ROW_STRIPE: Color32 = Color32::from_rgb(30, 32, 38);
const HEADER_FILL: Color32 = Color32::from_rgb(34, 36, 42);
const DANGER_FILL: Color32 = Color32::from_rgb(92, 42, 46);
const DANGER_TEXT: Color32 = Color32::from_rgb(255, 186, 186);
const FONT_BODY: f32 = 14.5;
const FONT_HEADER: f32 = 13.5;
const FONT_PILL: f32 = 13.0;
const ROW_H: f32 = 36.0;
const ROW_INSET: f32 = 12.0;
const TABLE_BOTTOM_GAP: f32 = 12.0;
const AUTHOR_NAME: &str = "thuanlm215";
const AUTHOR_URL: &str = "https://github.com/thuanlm215/autotun";

pub fn run(cli: &Cli) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("autotun")
            .with_inner_size([960.0, 560.0])
            .with_min_inner_size([840.0, 420.0]),
        ..Default::default()
    };
    let app = GuiApp::from_cli(cli);
    eframe::run_native(
        "autotun",
        options,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
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
    clip_notice: Option<ClipNotice>,
}

enum ClipNotice {
    Success(String),
    Error(String),
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
                    clip_notice: None,
                });
            }
            Err(error) => self.connect_error = Some(format!("{error:#}")),
        }
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));
        if let Some(session) = &self.session {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "autotun — {}",
                session.engine.destination()
            )));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title("autotun".into()));
        }
        egui::TopBottomPanel::bottom("credits")
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(Margin {
                left: 16,
                right: 20,
                top: 4,
                bottom: 10,
            }))
            .show(ctx, |ui| {
                ui.vertical_centered(author_footer);
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ctx.style()).inner_margin(Margin {
                    left: 16,
                    right: 20,
                    top: 14,
                    bottom: 18,
                }),
            )
            .show(ctx, |ui| {
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
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add_space(36.0);
            ui.heading(RichText::new("autotun").size(26.0));
            ui.label(
                RichText::new("Connect over SSH and manage port forwards.")
                    .color(MUTED)
                    .size(14.5),
            );
            ui.add_space(22.0);

            let inner_width = 460.0_f32.min(ui.available_width() - 8.0);
            card().show(ui, |ui| {
                ui.set_width(inner_width);
                ui.spacing_mut().item_spacing = Vec2::new(12.0, 10.0);
                egui::Grid::new("connect")
                    .num_columns(2)
                    .spacing([12.0, 10.0])
                    .min_col_width(120.0)
                    .show(ui, |ui| {
                        ui.label("Destination");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.destination)
                                .desired_width(f32::INFINITY)
                                .hint_text("user@host or SSH alias"),
                        );
                        ui.end_row();

                        ui.label("Reverse ports");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.reverse_text)
                                .desired_width(f32::INFINITY)
                                .hint_text("optional, e.g. 3000, 8080"),
                        );
                        ui.end_row();

                        ui.label("Extra SSH args");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.ssh_args_text)
                                .desired_width(f32::INFINITY)
                                .hint_text("optional, e.g. -J bastion"),
                        );
                        ui.end_row();

                        ui.label("Scan interval");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.interval_text)
                                    .desired_width(64.0),
                            );
                            ui.label(RichText::new("seconds").color(MUTED));
                        });
                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                    ui.set_width(inner_width);
                    ui.checkbox(
                        &mut self.auto_forward,
                        "Auto-forward discovered remote ports",
                    );
                    ui.checkbox(
                        &mut self.include_loopback,
                        "Include remote loopback listeners",
                    );
                });
                ui.add_space(14.0);
                if ui
                    .add_sized([inner_width, 34.0], primary_button("Connect"))
                    .clicked()
                {
                    self.try_connect();
                }
                if let Some(error) = &self.connect_error {
                    ui.add_space(8.0);
                    ui.colored_label(ERR_COLOR, error);
                }
            });
        });
    }

    fn session_ui(&mut self, ui: &mut egui::Ui) {
        let mut disconnect = false;
        {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            session.engine.poll();
            header_bar(ui, session, &mut disconnect);
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

        ui.add_space(10.0);
        clip_banner(ui, session);
        if session.clip_notice.is_some() {
            ui.add_space(8.0);
        }

        let mut save_form = false;
        let mut cancel_form = false;
        if let Some(form) = session.form.as_mut() {
            card().show(ui, |ui| {
                ui.label(RichText::new(&form.title).strong());
                ui.add_space(6.0);
                let (source_label, requested_label) = match form.direction {
                    Direction::Local => ("Remote port", "Local port (optional)"),
                    Direction::Reverse => ("Local port", "Remote port (optional)"),
                };
                ui.horizontal(|ui| {
                    ui.label(source_label);
                    ui.add(egui::TextEdit::singleline(&mut form.source).desired_width(80.0));
                    ui.add_space(8.0);
                    ui.label(requested_label);
                    ui.add(
                        egui::TextEdit::singleline(&mut form.requested)
                            .desired_width(80.0)
                            .hint_text("same"),
                    );
                    ui.add_space(8.0);
                    ui.label("Label");
                    ui.add(egui::TextEdit::singleline(&mut form.label).desired_width(140.0));
                    ui.add_space(8.0);
                    save_form = ui.add(primary_button("Save")).clicked();
                    cancel_form = ui.button("Cancel").clicked();
                });
            });
            if let Some(error) = &session.form_error {
                ui.add_space(6.0);
                ui.colored_label(ERR_COLOR, error);
            }
            ui.add_space(8.0);
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
            .filter(|(_, tunnel)| tunnel_matches(tunnel, &filter))
            .map(|(index, _)| index)
            .collect();
        tunnel_table(ui, session, &visible);
    }
}

fn header_bar(ui: &mut egui::Ui, session: &mut SessionUi, disconnect: &mut bool) {
    ui.horizontal(|ui| {
        ui.heading("autotun");
        ui.add_space(6.0);
        ui.label(
            RichText::new(session.engine.destination())
                .strong()
                .size(16.0),
        );
        if session.engine.connected() {
            status_dot(ui, ON_COLOR, "connected");
        } else {
            status_dot(ui, WARN_COLOR, "reconnecting");
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("Disconnect").color(DANGER_TEXT))
                        .fill(DANGER_FILL),
                )
                .clicked()
            {
                *disconnect = true;
            }
            ui.add_space(10.0);
            if ui.button("Rescan").clicked() {
                session.engine.rescan();
            }
            if ui
                .button("Send screenshot")
                .on_hover_text("Upload the clipboard PNG to the remote host")
                .clicked()
            {
                match session.engine.push_clipboard_image() {
                    Ok(path) => session.clip_notice = Some(ClipNotice::Success(path)),
                    Err(error) => {
                        session.clip_notice = Some(ClipNotice::Error(format!("{error:#}")))
                    }
                }
            }
            ui.add_space(10.0);
            if ui.add(primary_button("Reverse")).clicked() {
                session.form = Some(FormUi::new(Direction::Reverse));
                session.form_error = None;
            }
            if ui.add(primary_button("Forward")).clicked() {
                session.form = Some(FormUi::new(Direction::Local));
                session.form_error = None;
            }
        });
    });
}

fn filter_bar(ui: &mut egui::Ui, session: &mut SessionUi) {
    let filter = session.filter.to_lowercase();
    let total = session.engine.tunnels().len();
    let shown = session
        .engine
        .tunnels()
        .iter()
        .filter(|tunnel| tunnel_matches(tunnel, &filter))
        .count();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Filter").color(MUTED).size(FONT_HEADER));
        ui.add(
            egui::TextEdit::singleline(&mut session.filter)
                .desired_width(280.0)
                .hint_text("label or port"),
        );
        ui.label(
            RichText::new(format!("{shown} / {total}"))
                .color(MUTED)
                .size(FONT_HEADER),
        );
    });
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

fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);
    style.spacing.interact_size.y = 26.0;
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(12.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Body,
        FontId::new(FONT_BODY, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(22.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(14.0, FontFamily::Monospace),
    );

    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(Color32::from_rgb(214, 218, 224));
    visuals.hyperlink_color = Color32::from_rgb(122, 186, 232);
    visuals.warn_fg_color = WARN_COLOR;
    visuals.error_fg_color = ERR_COLOR;
    visuals.panel_fill = Color32::from_rgb(22, 24, 28);
    visuals.window_fill = CARD_FILL;
    visuals.extreme_bg_color = Color32::from_rgb(16, 17, 21);
    visuals.faint_bg_color = ROW_STRIPE;
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(5);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(5);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(5);
    visuals.widgets.active.corner_radius = CornerRadius::same(5);
    visuals.widgets.open.corner_radius = CornerRadius::same(5);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(48, 52, 60);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(64, 70, 80);
    visuals.widgets.active.bg_fill = Color32::from_rgb(56, 62, 72);
    visuals.selection.bg_fill = Color32::from_rgb(38, 110, 122);
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(110, 196, 204));
    style.visuals = visuals;
    ctx.set_style(style);
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD_FILL)
        .stroke(Stroke::new(1.0_f32, CARD_STROKE))
        .corner_radius(8)
        .inner_margin(Margin::same(16))
}

fn primary_button(label: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.to_owned()).color(Color32::from_rgb(240, 248, 248)))
        .fill(ACCENT)
}

fn author_footer(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new("by").color(MUTED).size(13.0));
        ui.hyperlink_to(
            RichText::new(AUTHOR_NAME).color(CLIP_COLOR).size(13.0),
            AUTHOR_URL,
        );
    });
}

fn status_dot(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.5, color);
        ui.label(RichText::new(text).color(color));
    });
}

fn clip_banner(ui: &mut egui::Ui, session: &mut SessionUi) {
    let Some(notice) = &session.clip_notice else {
        return;
    };
    let (fill, stroke) = match notice {
        ClipNotice::Success(_) => (
            Color32::from_rgb(28, 42, 48),
            Color32::from_rgb(48, 90, 104),
        ),
        ClipNotice::Error(_) => (
            Color32::from_rgb(48, 30, 32),
            Color32::from_rgb(110, 56, 58),
        ),
    };
    let mut dismiss = false;
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, stroke))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                match notice {
                    ClipNotice::Success(path) => {
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("{path}  (copied — paste in the AI CLI)"))
                                    .color(CLIP_COLOR)
                                    .monospace(),
                            )
                            .selectable(true),
                        );
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(path.clone());
                        }
                    }
                    ClipNotice::Error(error) => {
                        ui.add(
                            egui::Label::new(RichText::new(error).color(ERR_COLOR))
                                .selectable(true),
                        );
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Dismiss").clicked() {
                        dismiss = true;
                    }
                });
            });
        });
    if dismiss {
        session.clip_notice = None;
    }
}

fn tunnel_table(ui: &mut egui::Ui, session: &mut SessionUi, visible: &[usize]) {
    let remaining = ui.available_height();
    egui::Frame::new()
        .fill(TABLE_FILL)
        .stroke(Stroke::new(1.0_f32, CARD_STROKE))
        .corner_radius(8)
        .inner_margin(Margin::same(10))
        .outer_margin(Margin {
            left: 0,
            right: 12,
            top: 0,
            bottom: TABLE_BOTTOM_GAP as i8,
        })
        .show(ui, |ui| {
            ui.set_min_height((remaining - TABLE_BOTTOM_GAP - 8.0).max(140.0));
            filter_bar(ui, session);
            ui.add_space(6.0);
            let widths = col_widths(ui.available_width());
            header_row(ui, &widths);
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if visible.is_empty() {
                        ui.add_space(24.0);
                        ui.vertical_centered(|ui| {
                            let empty = if session.filter.is_empty() {
                                "No tunnels yet"
                            } else {
                                "No tunnels match this filter"
                            };
                            ui.label(RichText::new(empty).color(MUTED));
                        });
                        return;
                    }
                    let mut action = None::<RowAction>;
                    for (row_i, &index) in visible.iter().enumerate() {
                        let tunnel = &session.engine.tunnels()[index];
                        paint_row_bg(ui, row_i);
                        ui.horizontal(|ui| {
                            ui.set_height(ROW_H);
                            ui.add_space(ROW_INSET);
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
                            let (status, status_color) = tunnel_status(tunnel);
                            let url = engine::tunnel_url(tunnel);
                            let label = if tunnel.label.is_empty() {
                                "—"
                            } else {
                                tunnel.label.as_str()
                            };

                            cell(ui, widths[0], |ui| {
                                ui.label(direction);
                            });
                            cell(ui, widths[1], |ui| {
                                ui.label(label);
                            });
                            cell(ui, widths[2], |ui| {
                                ui.monospace(remote);
                            });
                            cell(ui, widths[3], |ui| {
                                ui.monospace(local);
                            });
                            cell(ui, widths[4], |ui| {
                                if url == "—" {
                                    ui.label(RichText::new("—").color(MUTED));
                                } else if ui.link(&url).clicked() {
                                    let _ =
                                        std::process::Command::new("xdg-open").arg(&url).spawn();
                                }
                            });
                            cell(ui, widths[5], |ui| {
                                status_pill(ui, &status, status_color);
                            });
                            cell(ui, widths[6], |ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                let toggle_label = if tunnel.enabled { "Off" } else { "On" };
                                if ui.button(toggle_label).clicked() {
                                    action = Some(RowAction::Toggle(index));
                                }
                                if ui.button("Edit").clicked() {
                                    action = Some(RowAction::Edit(index));
                                }
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Remove").color(DANGER_TEXT),
                                        )
                                        .fill(DANGER_FILL),
                                    )
                                    .clicked()
                                {
                                    action = Some(RowAction::Delete(index));
                                }
                            });
                        });
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

fn header_row(ui: &mut egui::Ui, widths: &[f32; 7]) {
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), ROW_H));
    ui.painter()
        .rect_filled(rect, CornerRadius::same(5), HEADER_FILL);
    ui.horizontal(|ui| {
        ui.set_height(ROW_H);
        ui.add_space(ROW_INSET);
        for (i, title) in ["Direction", "Label", "Remote", "Local", "URL", "Status", ""]
            .into_iter()
            .enumerate()
        {
            cell(ui, widths[i], |ui| {
                if i == 6 {
                    return;
                }
                ui.label(
                    RichText::new(title)
                        .strong()
                        .size(FONT_HEADER)
                        .color(Color32::from_rgb(186, 192, 200)),
                );
            });
        }
    });
}

fn paint_row_bg(ui: &mut egui::Ui, row_i: usize) {
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), ROW_H));
    if ui.rect_contains_pointer(rect) {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), ROW_HOVER);
    } else if row_i % 2 == 1 {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(5), ROW_STRIPE);
    }
}

fn cell(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, ui.available_height()),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            add(ui);
        },
    );
}

fn col_widths(available: f32) -> [f32; 7] {
    // Keep columns clustered. Extra window width stays to the right of Actions.
    let mut widths = [92.0, 120.0, 76.0, 76.0, 214.0, 132.0, 196.0];
    let spacing = 6.0 * 6.0;
    let total: f32 = widths.iter().sum::<f32>() + spacing;
    if total <= available {
        return widths;
    }
    let overflow = total - available;
    let url_shrink = overflow.min((widths[4] - 150.0).max(0.0));
    widths[4] -= url_shrink;
    let leftover = overflow - url_shrink;
    if leftover > 0.0 {
        widths[1] = (widths[1] - leftover).max(84.0);
    }
    widths
}

fn tunnel_matches(tunnel: &crate::ports::Tunnel, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    tunnel.label.to_lowercase().contains(filter)
        || tunnel.source_port.to_string().contains(filter)
        || tunnel
            .bind_port
            .map(|port| port.to_string().contains(filter))
            .unwrap_or(false)
}

fn tunnel_status(tunnel: &crate::ports::Tunnel) -> (String, Color32) {
    if let Some(error) = &tunnel.error {
        return (error.clone(), ERR_COLOR);
    }
    if tunnel.enabled {
        ("ON".into(), ON_COLOR)
    } else if tunnel.manual_off {
        ("MANUAL OFF".into(), MUTED)
    } else if !tunnel.present {
        ("TARGET DOWN".into(), WARN_COLOR)
    } else {
        ("off".into(), MUTED)
    }
}

fn status_pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            28,
        ))
        .stroke(Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 90),
        ))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).size(FONT_PILL).strong());
        });
}
