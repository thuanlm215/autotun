//! Desktop frontend (egui). Compiled only with `--features gui`.

use std::time::Duration;

use anyhow::Result;
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontFamily, FontId, Layout, Margin, Rect, RichText, Stroke,
    StrokeKind, TextStyle, Vec2,
};
use serde::{Deserialize, Serialize};

use crate::{
    cli::Cli,
    engine::{self, Engine},
    ports::Direction,
    remote_app::{RemoteAppManager, RemoteAppStatus},
};

const FONT_BODY: f32 = 14.0;
const FONT_HEADER: f32 = 13.0;
const FONT_PILL: f32 = 12.0;
const ROW_H: f32 = 36.0;
const ROW_INSET: f32 = 12.0;
const TABLE_BOTTOM_GAP: f32 = 12.0;
const AUTHOR_NAME: &str = "thuanlm215";
const AUTHOR_URL: &str = "https://github.com/thuanlm215/autotun";
const CONNECT_PREFS_KEY: &str = "autotun.connect-preferences";
const APP_ICON: &[u8] = include_bytes!("../packaging/autotun.png");
const FONT_REGULAR_BYTES: &[u8] = include_bytes!("../packaging/fonts/LiberationSans-Regular.ttf");
const FONT_BOLD_BYTES: &[u8] = include_bytes!("../packaging/fonts/LiberationSans-Bold.ttf");

fn bold_family() -> FontFamily {
    FontFamily::Name(std::sync::Arc::from("bold"))
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "LiberationSans-Regular".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(FONT_REGULAR_BYTES)),
    );
    fonts.font_data.insert(
        "LiberationSans-Bold".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(FONT_BOLD_BYTES)),
    );

    if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        prop.insert(0, "LiberationSans-Regular".to_owned());
    }

    fonts.families.insert(
        egui::FontFamily::Name("bold".into()),
        vec![
            "LiberationSans-Bold".to_owned(),
            "LiberationSans-Regular".to_owned(),
        ],
    );

    ctx.set_fonts(fonts);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    Auto,
}

impl ThemeMode {
    pub fn is_dark(self, ctx: &egui::Context) -> bool {
        match self {
            Self::Dark => true,
            Self::Light => false,
            Self::Auto => match ctx.theme() {
                egui::Theme::Light => false,
                _ => true,
            },
        }
    }
}

impl From<ThemeMode> for egui::ThemePreference {
    fn from(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => egui::ThemePreference::Dark,
            ThemeMode::Light => egui::ThemePreference::Light,
            ThemeMode::Auto => egui::ThemePreference::System,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ThemePalette {
    pub is_dark: bool,
    pub panel_fill: Color32,
    pub card_fill: Color32,
    pub card_stroke: Color32,
    pub table_fill: Color32,
    pub row_stripe: Color32,
    pub row_hover: Color32,
    pub header_fill: Color32,
    pub header_text: Color32,
    pub badge_bg: Color32,
    pub text_primary: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub danger_fill: Color32,
    pub danger_stroke: Color32,
    pub danger_text: Color32,
    pub warn_color: Color32,
    pub err_color: Color32,
    pub on_color: Color32,
    pub clip_color: Color32,
}

impl ThemePalette {
    pub fn get(is_dark: bool) -> Self {
        if is_dark {
            Self {
                is_dark: true,
                panel_fill: Color32::from_rgb(15, 17, 22),
                card_fill: Color32::from_rgb(24, 27, 34),
                card_stroke: Color32::from_rgb(46, 54, 68),
                table_fill: Color32::from_rgb(20, 22, 28),
                row_stripe: Color32::from_rgb(25, 29, 37),
                row_hover: Color32::from_rgb(34, 40, 52),
                header_fill: Color32::from_rgb(30, 36, 46),
                header_text: Color32::from_rgb(203, 213, 225),
                badge_bg: Color32::from_rgb(32, 38, 48),
                text_primary: Color32::from_rgb(241, 245, 249),
                text_muted: Color32::from_rgb(148, 163, 184),
                accent: Color32::from_rgb(13, 148, 136),
                accent_hover: Color32::from_rgb(20, 184, 166),
                danger_fill: Color32::from_rgb(64, 22, 26),
                danger_stroke: Color32::from_rgb(153, 27, 27),
                danger_text: Color32::from_rgb(254, 202, 202),
                warn_color: Color32::from_rgb(250, 204, 21),
                err_color: Color32::from_rgb(248, 113, 113),
                on_color: Color32::from_rgb(74, 222, 128),
                clip_color: Color32::from_rgb(56, 189, 248),
            }
        } else {
            Self {
                is_dark: false,
                panel_fill: Color32::from_rgb(241, 245, 249),
                card_fill: Color32::from_rgb(255, 255, 255),
                card_stroke: Color32::from_rgb(203, 213, 225),
                table_fill: Color32::from_rgb(255, 255, 255),
                row_stripe: Color32::from_rgb(248, 250, 252),
                row_hover: Color32::from_rgb(241, 245, 249),
                header_fill: Color32::from_rgb(226, 232, 240),
                header_text: Color32::from_rgb(30, 41, 59),
                badge_bg: Color32::from_rgb(241, 245, 249),
                text_primary: Color32::from_rgb(15, 23, 42),
                text_muted: Color32::from_rgb(71, 85, 105),
                accent: Color32::from_rgb(15, 118, 110),
                accent_hover: Color32::from_rgb(13, 148, 136),
                danger_fill: Color32::from_rgb(254, 226, 226),
                danger_stroke: Color32::from_rgb(252, 165, 165),
                danger_text: Color32::from_rgb(185, 28, 28),
                warn_color: Color32::from_rgb(180, 83, 9),
                err_color: Color32::from_rgb(185, 28, 28),
                on_color: Color32::from_rgb(21, 128, 61),
                clip_color: Color32::from_rgb(2, 110, 189),
            }
        }
    }

    pub fn card(&self) -> egui::Frame {
        egui::Frame::new()
            .fill(self.card_fill)
            .stroke(Stroke::new(1.0_f32, self.card_stroke))
            .corner_radius(8)
            .inner_margin(Margin::same(16))
    }

    pub fn primary_button<'a>(&self, label: impl Into<String>) -> egui::Button<'a> {
        egui::Button::new(
            RichText::new(label.into())
                .color(Color32::from_rgb(240, 253, 250))
                .family(bold_family())
                .size(13.0),
        )
        .fill(self.accent)
        .corner_radius(6)
    }

    pub fn secondary_button<'a>(&self, label: impl Into<String>) -> egui::Button<'a> {
        egui::Button::new(
            RichText::new(label.into())
                .color(self.text_primary)
                .family(bold_family())
                .size(13.0),
        )
        .fill(self.badge_bg)
        .stroke(Stroke::new(1.0_f32, self.card_stroke))
        .corner_radius(6)
    }

    pub fn danger_button<'a>(&self, label: impl Into<String>) -> egui::Button<'a> {
        egui::Button::new(
            RichText::new(label.into())
                .color(self.danger_text)
                .family(bold_family())
                .size(12.5),
        )
        .fill(self.danger_fill)
        .stroke(Stroke::new(1.0_f32, self.danger_stroke))
        .corner_radius(6)
    }

    pub fn hint(&self, text: &str) -> RichText {
        RichText::new(text).color(if self.is_dark {
            Color32::from_rgba_unmultiplied(148, 163, 184, 140)
        } else {
            Color32::from_rgba_unmultiplied(100, 116, 139, 170)
        })
    }
}

pub fn run(cli: &Cli) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("autotun")
            .with_app_id("autotun")
            .with_inner_size([960.0, 560.0])
            .with_min_inner_size([840.0, 420.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(APP_ICON)
                    .expect("packaging/autotun.png is a valid PNG"),
            ),
        ..Default::default()
    };
    let cli = cli.clone();
    eframe::run_native(
        "autotun",
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            let app = GuiApp::from_cli(&cli, cc.storage);
            cc.egui_ctx.set_theme(app.theme_mode);
            let is_dark = app.theme_mode.is_dark(&cc.egui_ctx);
            let theme = ThemePalette::get(is_dark);
            apply_theme(&cc.egui_ctx, &theme);
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
    connect_icon: Option<egui::TextureHandle>,
    theme_mode: ThemeMode,
    session: Option<SessionUi>,
}

#[derive(Default, Deserialize, Serialize)]
struct ConnectPreferences {
    destination: String,
    reverse_text: String,
    ssh_args_text: String,
    #[serde(default)]
    theme_mode: ThemeMode,
}

struct SessionUi {
    engine: Engine,
    remote_apps: RemoteAppManager,
    remote_command: String,
    remote_app_error: Option<String>,
    page: SessionPage,
    filter: String,
    form: Option<FormUi>,
    form_error: Option<String>,
    clip_notice: Option<ClipNotice>,
    open_path: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionPage {
    Tunnels,
    RemoteApps,
}

impl Drop for SessionUi {
    fn drop(&mut self) {
        // Waypipe owns an SSH client using the ControlMaster. Stop those
        // children before Engine drops its master connection.
        self.remote_apps.stop_all();
        self.engine.shutdown();
    }
}

enum ClipNotice {
    Success(String),
    Opened(String),
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
    fn from_cli(cli: &Cli, storage: Option<&dyn eframe::Storage>) -> Self {
        let saved = storage
            .and_then(|storage| eframe::get_value::<ConnectPreferences>(storage, CONNECT_PREFS_KEY))
            .unwrap_or_default();
        let use_saved =
            cli.destination.is_none() && cli.reverse_ports.is_empty() && cli.ssh_args.is_empty();
        let reverse_text = cli
            .reverse_ports
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let ssh_args_text = cli.ssh_args.join(" ");
        let mut app = Self {
            destination: if use_saved {
                saved.destination
            } else {
                cli.destination.clone().unwrap_or_default()
            },
            reverse_text: if use_saved {
                saved.reverse_text
            } else {
                reverse_text
            },
            ssh_args_text: if use_saved {
                saved.ssh_args_text
            } else {
                ssh_args_text
            },
            interval_text: cli.interval.to_string(),
            include_loopback: cli.include_loopback,
            auto_forward: !cli.no_auto_forward,
            connect_error: None,
            connect_icon: None,
            theme_mode: saved.theme_mode,
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
                self.connect_error = Some("Scan interval must be a number of seconds >= 1.".into());
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
                    remote_apps: RemoteAppManager::default(),
                    remote_command: String::new(),
                    remote_app_error: None,
                    page: SessionPage::Tunnels,
                    filter: String::new(),
                    form: None,
                    form_error: None,
                    clip_notice: None,
                    open_path: String::new(),
                });
            }
            Err(error) => self.connect_error = Some(format!("{error:#}")),
        }
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));
        ctx.set_theme(self.theme_mode);
        let is_dark = self.theme_mode.is_dark(ctx);
        let theme = ThemePalette::get(is_dark);
        apply_theme(ctx, &theme);

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
            .frame(
                egui::Frame::new()
                    .fill(theme.panel_fill)
                    .inner_margin(Margin {
                        left: 16,
                        right: 20,
                        top: 4,
                        bottom: 8,
                    }),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    author_footer(ui, &theme);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        theme_switcher(ui, &mut self.theme_mode, &theme);
                    });
                });
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
                    self.session_ui(ui, &theme);
                } else {
                    self.connect_ui(ui, &theme);
                }
            });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(
            storage,
            CONNECT_PREFS_KEY,
            &ConnectPreferences {
                destination: self.destination.clone(),
                reverse_text: self.reverse_text.clone(),
                ssh_args_text: self.ssh_args_text.clone(),
                theme_mode: self.theme_mode,
            },
        );
    }
}

impl GuiApp {
    fn connect_ui(&mut self, ui: &mut egui::Ui, theme: &ThemePalette) {
        let icon = self
            .connect_icon
            .get_or_insert_with(|| {
                ui.ctx().load_texture(
                    "autotun-connect-icon",
                    app_icon_image(),
                    egui::TextureOptions::LINEAR,
                )
            })
            .clone();
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add_space(20.0);
            ui.image((icon.id(), Vec2::splat(60.0)));
            ui.add_space(8.0);
            ui.heading(
                RichText::new("autotun")
                    .size(24.0)
                    .strong()
                    .color(theme.text_primary),
            );
            ui.label(
                RichText::new("Connect over SSH and manage port forwards.")
                    .color(theme.text_muted)
                    .size(13.5),
            );
            ui.add_space(18.0);

            let inner_width = 460.0_f32.min(ui.available_width() - 8.0);
            theme.card().show(ui, |ui| {
                ui.set_width(inner_width);
                ui.spacing_mut().item_spacing = Vec2::new(12.0, 10.0);
                egui::Grid::new("connect")
                    .num_columns(2)
                    .spacing([12.0, 10.0])
                    .min_col_width(120.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Destination").strong().size(13.0));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.destination)
                                .desired_width(f32::INFINITY)
                                .hint_text(theme.hint("user@host or SSH alias")),
                        );
                        ui.end_row();

                        ui.label(RichText::new("Reverse ports").strong().size(13.0));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.reverse_text)
                                .desired_width(f32::INFINITY)
                                .hint_text(theme.hint("optional, e.g. 3000, 8080")),
                        );
                        ui.end_row();

                        ui.label(RichText::new("Extra SSH args").strong().size(13.0));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.ssh_args_text)
                                .desired_width(f32::INFINITY)
                                .hint_text(theme.hint("optional, e.g. -J bastion")),
                        );
                        ui.end_row();

                        ui.label(RichText::new("Scan interval").strong().size(13.0));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.interval_text)
                                    .desired_width(64.0),
                            );
                            ui.label(RichText::new("seconds").color(theme.text_muted));
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
                    .add_sized([inner_width, 34.0], theme.primary_button("Connect"))
                    .clicked()
                {
                    self.try_connect();
                }
                if let Some(error) = &self.connect_error {
                    ui.add_space(8.0);
                    ui.colored_label(theme.err_color, error);
                }
            });
        });
    }

    fn session_ui(&mut self, ui: &mut egui::Ui, theme: &ThemePalette) {
        let mut disconnect = false;
        {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            session.engine.poll();
            session.remote_apps.poll();
            header_bar(ui, session, &mut disconnect, theme);
            if disconnect {
                session.remote_apps.stop_all();
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

        ui.add_space(12.0);
        session_tabs(ui, session, theme);
        if session.clip_notice.is_some() {
            ui.add_space(8.0);
            clip_banner(ui, session, theme);
        }
        ui.add_space(12.0);
        match session.page {
            SessionPage::Tunnels => tunnels_panel(ui, session, theme),
            SessionPage::RemoteApps => remote_apps_panel(ui, session, theme),
        }
    }
}

fn tunnels_panel(ui: &mut egui::Ui, session: &mut SessionUi, theme: &ThemePalette) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.heading(
                RichText::new("Tunnels")
                    .strong()
                    .color(theme.text_primary)
                    .size(18.0),
            );
            ui.label(
                RichText::new("Manage forwarded and reverse ports.")
                    .color(theme.text_muted)
                    .size(12.5),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(theme.primary_button("+ Forward")).clicked() {
                session.form = Some(FormUi::new(Direction::Local));
                session.form_error = None;
            }
            if ui.add(theme.secondary_button("+ Reverse")).clicked() {
                session.form = Some(FormUi::new(Direction::Reverse));
                session.form_error = None;
            }
            if ui
                .add(theme.secondary_button("Rescan"))
                .on_hover_text("Query remote host for new listening ports")
                .clicked()
            {
                session.engine.rescan();
            }
        });
    });
    ui.add_space(8.0);

    let mut save_form = false;
    let mut cancel_form = false;
    if let Some(form) = session.form.as_mut() {
        theme.card().show(ui, |ui| {
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
                        .hint_text(theme.hint("same")),
                );
                ui.add_space(8.0);
                ui.label("Label");
                ui.add(egui::TextEdit::singleline(&mut form.label).desired_width(140.0));
                ui.add_space(8.0);
                save_form = ui.add(theme.primary_button("Save")).clicked();
                cancel_form = ui.button("Cancel").clicked();
            });
        });
        if let Some(error) = &session.form_error {
            ui.add_space(6.0);
            ui.colored_label(theme.err_color, error);
        }
        ui.add_space(8.0);
    }
    if cancel_form {
        session.form = None;
        session.form_error = None;
    } else if save_form && let Some(form) = session.form.take() {
        match engine::tunnel_from_form(form.direction, &form.source, &form.requested, &form.label) {
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
    tunnel_table(ui, session, &visible, theme);
}

fn remote_apps_panel(ui: &mut egui::Ui, session: &mut SessionUi, theme: &ThemePalette) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.heading(
                RichText::new("Remote Apps")
                    .strong()
                    .color(theme.text_primary)
                    .size(18.0),
            );
            ui.label(
                RichText::new(
                    "Open a remote file on this machine, or launch a Wayland app through Waypipe.",
                )
                .color(theme.text_muted)
                .size(12.5),
            );
        });
    });
    ui.add_space(8.0);
    open_file_card(ui, session, theme);
    ui.add_space(10.0);
    theme.card().show(ui, |ui| {
        ui.label(RichText::new("Launch app").strong().size(13.0));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Command").strong().size(13.0));
            let response = ui.add(
                egui::TextEdit::singleline(&mut session.remote_command)
                    .desired_width(340.0)
                    .hint_text(theme.hint("e.g. firefox --new-instance")),
            );
            let launch = ui.add(theme.primary_button("Launch")).clicked()
                || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            if launch {
                let control = session.engine.ssh_control();
                match session.remote_apps.launch(&session.remote_command, control) {
                    Ok(()) => {
                        session.remote_command.clear();
                        session.remote_app_error = None;
                    }
                    Err(error) => session.remote_app_error = Some(format!("{error:#}")),
                }
            }
            if session.remote_apps.has_finished() && ui.link("Clear history").clicked() {
                session.remote_apps.clear_finished();
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Presets:").color(theme.text_muted).size(12.0));
            let presets = [
                ("Firefox", "firefox --new-instance"),
                ("Files", "nautilus"),
                ("Gedit", "gedit"),
                ("Terminal", "kitty"),
            ];
            for (title, cmd) in presets {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(title)
                                .size(11.5)
                                .color(theme.text_primary),
                        )
                        .fill(theme.badge_bg)
                        .corner_radius(4),
                    )
                    .on_hover_text(format!("Fill command: {cmd}"))
                    .clicked()
                {
                    session.remote_command = cmd.to_owned();
                }
            }
        });

        if let Some(error) = &session.remote_app_error {
            ui.add_space(6.0);
            ui.colored_label(theme.err_color, error);
        }

        let apps = session.remote_apps.apps();
        if apps.is_empty() {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("No remote applications launched yet")
                        .color(theme.text_primary)
                        .size(13.5),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Requires a local Wayland session and waypipe installed on both host and VM.")
                        .color(theme.text_muted)
                        .size(12.0),
                );
            });
            return;
        }
        ui.add_space(5.0);
        let mut stop = None;
        for app in apps {
            let (status_label, status_kind, details) = match &app.status {
                RemoteAppStatus::Starting => ("Starting", StatusKind::Warn, app.status.details()),
                RemoteAppStatus::Running => ("Running", StatusKind::On, app.status.details()),
                RemoteAppStatus::Exited(_) => ("Exited", StatusKind::Off, app.status.details()),
                RemoteAppStatus::Failed(_) => ("Failed", StatusKind::Error, app.status.details()),
            };
            ui.horizontal(|ui| {
                ui.monospace(&app.command);
                status_pill(ui, status_label, theme, status_kind, details);
                if app.status.can_stop() && ui.button("Stop").clicked() {
                    stop = Some(app.id);
                }
            });
            if let Some(details) = app.status.details() {
                let detail_color = match status_kind {
                    StatusKind::Error => theme.err_color,
                    StatusKind::Warn => theme.warn_color,
                    StatusKind::On => theme.on_color,
                    StatusKind::Off => theme.text_muted,
                };
                ui.label(RichText::new(remote_app_summary(details)).color(detail_color));
                ui.push_id(app.id, |ui| {
                    ui.collapsing("Details", |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(details).color(detail_color)).selectable(true),
                        );
                    });
                });
            }
        }
        if let Some(id) = stop {
            session.remote_apps.stop(id);
        }
    });
}

fn session_tabs(ui: &mut egui::Ui, session: &mut SessionUi, theme: &ThemePalette) {
    let active_tunnels = session
        .engine
        .tunnels()
        .iter()
        .filter(|t| t.enabled)
        .count();
    let total_tunnels = session.engine.tunnels().len();
    let running_apps = session
        .remote_apps
        .apps()
        .iter()
        .filter(|a| matches!(a.status, RemoteAppStatus::Running))
        .count();

    let tab_bg = if theme.is_dark {
        Color32::from_rgb(18, 20, 26)
    } else {
        Color32::from_rgb(235, 240, 246)
    };
    egui::Frame::new()
        .fill(tab_bg)
        .stroke(Stroke::new(1.0_f32, theme.card_stroke))
        .corner_radius(7)
        .inner_margin(Margin::symmetric(3, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let tunnels_selected = session.page == SessionPage::Tunnels;
                let (fill, stroke, text_color) = if tunnels_selected {
                    if theme.is_dark {
                        (
                            Color32::from_rgb(38, 46, 60),
                            Stroke::new(1.0_f32, Color32::from_rgb(64, 76, 96)),
                            Color32::WHITE,
                        )
                    } else {
                        (
                            Color32::WHITE,
                            Stroke::new(1.0_f32, theme.card_stroke),
                            theme.text_primary,
                        )
                    }
                } else {
                    (Color32::TRANSPARENT, Stroke::NONE, theme.text_muted)
                };
                let tunnels_title = format!("Tunnels ({active_tunnels}/{total_tunnels})");
                let resp = ui.add(
                    egui::Button::new(
                        RichText::new(tunnels_title)
                            .color(text_color)
                            .family(bold_family())
                            .size(13.0),
                    )
                    .fill(fill)
                    .stroke(stroke)
                    .corner_radius(5),
                );
                if resp.clicked() {
                    session.page = SessionPage::Tunnels;
                }

                let apps_selected = session.page == SessionPage::RemoteApps;
                let (fill, stroke, text_color) = if apps_selected {
                    if theme.is_dark {
                        (
                            Color32::from_rgb(38, 46, 60),
                            Stroke::new(1.0_f32, Color32::from_rgb(64, 76, 96)),
                            Color32::WHITE,
                        )
                    } else {
                        (
                            Color32::WHITE,
                            Stroke::new(1.0_f32, theme.card_stroke),
                            theme.text_primary,
                        )
                    }
                } else {
                    (Color32::TRANSPARENT, Stroke::NONE, theme.text_muted)
                };
                let apps_title = if running_apps > 0 {
                    format!("Remote Apps ({running_apps})")
                } else {
                    "Remote Apps".into()
                };
                let resp = ui.add(
                    egui::Button::new(
                        RichText::new(apps_title)
                            .color(text_color)
                            .family(bold_family())
                            .size(13.0),
                    )
                    .fill(fill)
                    .stroke(stroke)
                    .corner_radius(5),
                );
                if resp.clicked() {
                    session.page = SessionPage::RemoteApps;
                }
            });
        });
}

fn remote_app_summary(details: &str) -> String {
    let preferred = [
        "Failed to execvp",
        "Failed initializing GTK",
        "Permission denied",
    ]
    .into_iter()
    .find_map(|needle| details.find(needle).map(|index| &details[index..]))
    .or_else(|| details.lines().find(|line| !line.trim().is_empty()))
    .unwrap_or(details)
    .trim();
    const LIMIT: usize = 112;
    if preferred.chars().count() > LIMIT {
        format!(
            "{}...",
            preferred.chars().take(LIMIT - 3).collect::<String>()
        )
    } else {
        preferred.to_owned()
    }
}

fn header_bar(
    ui: &mut egui::Ui,
    session: &mut SessionUi,
    disconnect: &mut bool,
    theme: &ThemePalette,
) {
    ui.horizontal(|ui| {
        ui.heading(
            RichText::new("autotun")
                .family(bold_family())
                .color(theme.text_primary)
                .size(20.0),
        );
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(theme.badge_bg)
            .stroke(Stroke::new(1.0_f32, theme.card_stroke))
            .corner_radius(6)
            .inner_margin(Margin::symmetric(9, 3))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(
                        RichText::new(session.engine.destination())
                            .family(bold_family())
                            .color(theme.text_primary)
                            .size(13.5),
                    );
                    let (dot_color, text) = if session.engine.connected() {
                        (theme.on_color, "connected")
                    } else {
                        (theme.warn_color, "reconnecting")
                    };
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 3.0, dot_color);
                    ui.label(
                        RichText::new(text)
                            .family(bold_family())
                            .color(dot_color)
                            .size(12.0),
                    );
                });
            });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(theme.danger_button("Disconnect")).clicked() {
                *disconnect = true;
            }
            if ui
                .add(theme.secondary_button("Send Image"))
                .on_hover_text("Upload clipboard image to remote host (copies path for CLI)")
                .clicked()
            {
                match session.engine.push_clipboard_image() {
                    Ok(path) => session.clip_notice = Some(ClipNotice::Success(path)),
                    Err(error) => {
                        session.clip_notice = Some(ClipNotice::Error(format!("{error:#}")))
                    }
                }
            }
        });
    });
}

fn filter_bar(ui: &mut egui::Ui, session: &mut SessionUi, theme: &ThemePalette) {
    let filter = session.filter.to_lowercase();
    let total = session.engine.tunnels().len();
    let shown = session
        .engine
        .tunnels()
        .iter()
        .filter(|tunnel| tunnel_matches(tunnel, &filter))
        .count();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut session.filter)
                .desired_width(240.0)
                .hint_text(theme.hint("Search port or label...")),
        );
        if !session.filter.is_empty()
            && ui.small_button("×").on_hover_text("Clear filter").clicked()
        {
            session.filter.clear();
        }
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(theme.badge_bg)
            .stroke(Stroke::new(1.0_f32, theme.card_stroke))
            .corner_radius(4)
            .inner_margin(Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!("{shown} / {total}"))
                        .color(theme.text_muted)
                        .size(12.0)
                        .family(bold_family()),
                );
            });
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
                Direction::Local => "Add forward (remote -> local)".into(),
                Direction::Reverse => "Add reverse (local -> remote)".into(),
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
                Direction::Local => "Edit forward (remote -> local)".into(),
                Direction::Reverse => "Edit reverse (local -> remote)".into(),
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

fn apply_theme(ctx: &egui::Context, theme: &ThemePalette) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.interact_size.y = 28.0;
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(12.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Body,
        FontId::new(FONT_BODY, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Button, FontId::new(13.5, bold_family()));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::new(20.0, bold_family()));
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(13.5, FontFamily::Monospace),
    );

    let mut visuals = if theme.is_dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(theme.text_primary);
    visuals.hyperlink_color = theme.clip_color;
    visuals.warn_fg_color = theme.warn_color;
    visuals.error_fg_color = theme.err_color;
    visuals.panel_fill = theme.panel_fill;
    visuals.window_fill = theme.card_fill;
    visuals.extreme_bg_color = if theme.is_dark {
        Color32::from_rgb(12, 14, 18)
    } else {
        Color32::from_rgb(241, 245, 249)
    };
    visuals.faint_bg_color = theme.row_stripe;
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(6);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
    visuals.widgets.active.corner_radius = CornerRadius::same(6);
    visuals.widgets.open.corner_radius = CornerRadius::same(6);

    if theme.is_dark {
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, theme.card_stroke);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(30, 35, 45);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, theme.card_stroke);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 48, 62);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, theme.accent);
        visuals.widgets.active.bg_fill = Color32::from_rgb(48, 56, 72);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, theme.accent);
        visuals.selection.bg_fill = Color32::from_rgb(15, 76, 92);
        visuals.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(56, 189, 248));
    } else {
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, theme.card_stroke);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(241, 245, 249);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, theme.card_stroke);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(230, 236, 244);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, theme.accent);
        visuals.widgets.active.bg_fill = Color32::from_rgb(220, 228, 238);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, theme.accent);
        visuals.selection.bg_fill = Color32::from_rgb(204, 251, 241);
        visuals.selection.stroke = Stroke::new(1.0_f32, theme.accent);
    }
    style.visuals = visuals;
    ctx.set_style(style);
}

fn app_icon_image() -> egui::ColorImage {
    let icon =
        eframe::icon_data::from_png_bytes(APP_ICON).expect("packaging/autotun.png is a valid PNG");
    egui::ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize],
        &icon.rgba,
    )
}

fn author_footer(ui: &mut egui::Ui, theme: &ThemePalette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new("by").color(theme.text_muted).size(12.0));
        ui.hyperlink_to(
            RichText::new(AUTHOR_NAME)
                .color(theme.clip_color)
                .size(12.0),
            AUTHOR_URL,
        );
        ui.label(RichText::new("•").color(theme.card_stroke).size(12.0));
        ui.label(
            RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                .color(theme.text_muted)
                .size(12.0),
        );
    });
}

fn theme_switcher(ui: &mut egui::Ui, theme_mode: &mut ThemeMode, theme: &ThemePalette) {
    egui::Frame::new()
        .fill(if theme.is_dark {
            Color32::from_rgb(22, 25, 32)
        } else {
            Color32::from_rgb(229, 231, 235)
        })
        .stroke(Stroke::new(1.0_f32, theme.card_stroke))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(3, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (mode, label) in [
                    (ThemeMode::Dark, "Dark"),
                    (ThemeMode::Light, "Light"),
                    (ThemeMode::Auto, "Auto"),
                ] {
                    let selected = *theme_mode == mode;
                    let (fill, text_color) = if selected {
                        (theme.accent, Color32::from_rgb(240, 253, 250))
                    } else {
                        (Color32::TRANSPARENT, theme.text_muted)
                    };
                    let btn = egui::Button::new(
                        RichText::new(label).size(11.5).color(text_color).strong(),
                    )
                    .fill(fill)
                    .corner_radius(4);
                    if ui
                        .add(btn)
                        .on_hover_text(match mode {
                            ThemeMode::Dark => "Always use dark theme",
                            ThemeMode::Light => "Always use light theme",
                            ThemeMode::Auto => "Sync with system theme preference",
                        })
                        .clicked()
                    {
                        *theme_mode = mode;
                    }
                }
            });
        });
}

fn open_file_card(ui: &mut egui::Ui, session: &mut SessionUi, theme: &ThemePalette) {
    theme.card().show(ui, |ui| {
        ui.label(RichText::new("Open file").strong().size(13.0));
        ui.label(
            RichText::new(
                "Paste a remote path or file:// URL. Downloads over SSH and opens locally.",
            )
            .color(theme.text_muted)
            .size(12.0),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut session.open_path)
                    .desired_width((ui.available_width() - 88.0).max(200.0))
                    .hint_text(theme.hint("/home/you/.grok/sessions/.../images/1.jpg"))
                    .font(FontId::monospace(13.0)),
            );
            let enter =
                response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui
                .add(theme.primary_button("Open"))
                .on_hover_text("Download the remote file over SSH and open it locally")
                .clicked()
                || enter
            {
                let spec = session.open_path.clone();
                match session.engine.open_remote_file(&spec) {
                    Ok(path) => {
                        session.clip_notice = Some(ClipNotice::Opened(path.display().to_string()));
                        session.open_path.clear();
                    }
                    Err(error) => {
                        session.clip_notice = Some(ClipNotice::Error(format!("{error:#}")));
                    }
                }
            }
        });
    });
}

fn clip_banner(ui: &mut egui::Ui, session: &mut SessionUi, theme: &ThemePalette) {
    let Some(notice) = &session.clip_notice else {
        return;
    };
    let (fill, stroke, tag, tag_bg, text_color) = match notice {
        ClipNotice::Success(_) | ClipNotice::Opened(_) => {
            if theme.is_dark {
                (
                    Color32::from_rgb(20, 34, 42),
                    Color32::from_rgb(38, 86, 108),
                    match notice {
                        ClipNotice::Opened(_) => "OPENED",
                        _ => "COPIED",
                    },
                    Color32::from_rgb(14, 116, 144),
                    theme.clip_color,
                )
            } else {
                (
                    Color32::from_rgb(240, 249, 255),
                    Color32::from_rgb(186, 230, 253),
                    match notice {
                        ClipNotice::Opened(_) => "OPENED",
                        _ => "COPIED",
                    },
                    Color32::from_rgb(2, 132, 199),
                    Color32::from_rgb(3, 105, 161),
                )
            }
        }
        ClipNotice::Error(_) => {
            if theme.is_dark {
                (
                    Color32::from_rgb(44, 22, 26),
                    Color32::from_rgb(118, 44, 52),
                    "ERROR",
                    Color32::from_rgb(185, 28, 28),
                    theme.err_color,
                )
            } else {
                (
                    Color32::from_rgb(254, 242, 242),
                    Color32::from_rgb(254, 202, 202),
                    "ERROR",
                    Color32::from_rgb(220, 38, 38),
                    Color32::from_rgb(185, 28, 28),
                )
            }
        }
    };
    let mut dismiss = false;
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, stroke))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                egui::Frame::new()
                    .fill(tag_bg)
                    .corner_radius(4)
                    .inner_margin(Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.label(RichText::new(tag).color(Color32::WHITE).size(11.0).strong());
                    });
                match notice {
                    ClipNotice::Success(path) => {
                        ui.add(
                            egui::Label::new(
                                RichText::new(path).color(text_color).monospace().size(13.0),
                            )
                            .selectable(true),
                        );
                        ui.label(
                            RichText::new("(copied — paste in AI CLI)")
                                .color(theme.text_muted)
                                .size(12.0),
                        );
                        if ui.small_button("Copy again").clicked() {
                            ui.ctx().copy_text(path.clone());
                        }
                    }
                    ClipNotice::Opened(path) => {
                        ui.add(
                            egui::Label::new(
                                RichText::new(path).color(text_color).monospace().size(13.0),
                            )
                            .selectable(true),
                        );
                        ui.label(
                            RichText::new("(opened on this machine)")
                                .color(theme.text_muted)
                                .size(12.0),
                        );
                    }
                    ClipNotice::Error(error) => {
                        ui.add(
                            egui::Label::new(
                                RichText::new(error).color(theme.err_color).size(13.0),
                            )
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

fn tunnel_table(
    ui: &mut egui::Ui,
    session: &mut SessionUi,
    visible: &[usize],
    theme: &ThemePalette,
) {
    let remaining = ui.available_height();
    egui::Frame::new()
        .fill(theme.table_fill)
        .stroke(Stroke::new(1.0_f32, theme.card_stroke))
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
            filter_bar(ui, session, theme);
            ui.add_space(6.0);
            let widths = col_widths(ui.available_width());
            header_row(ui, &widths, theme);
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
                            ui.label(RichText::new(empty).color(theme.text_muted));
                        });
                        return;
                    }
                    let mut action = None::<RowAction>;
                    for (row_i, &index) in visible.iter().enumerate() {
                        let tunnel = &session.engine.tunnels()[index];
                        paint_row_bg(ui, row_i, theme);
                        ui.horizontal(|ui| {
                            ui.set_height(ROW_H);
                            ui.add_space(ROW_INSET);
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
                            let status = tunnel_status(tunnel);
                            let url = engine::tunnel_url(tunnel);
                            let label = if tunnel.label.is_empty() {
                                "—"
                            } else {
                                tunnel.label.as_str()
                            };

                            cell(ui, widths[0], |ui| {
                                direction_pill(ui, tunnel.direction, theme);
                            });
                            cell(ui, widths[1], |ui| {
                                if label == "—" {
                                    ui.label(RichText::new("—").color(theme.text_muted));
                                } else {
                                    ui.label(
                                        RichText::new(label).color(theme.text_primary).size(13.5),
                                    );
                                }
                            });
                            cell(ui, widths[2], |ui| {
                                ui.monospace(
                                    RichText::new(remote)
                                        .color(theme.text_primary)
                                        .family(bold_family())
                                        .size(13.0),
                                );
                            });
                            cell(ui, widths[3], |ui| {
                                ui.monospace(
                                    RichText::new(local)
                                        .color(theme.text_primary)
                                        .family(bold_family())
                                        .size(13.0),
                                );
                            });
                            cell(ui, widths[4], |ui| {
                                if url == "—" {
                                    ui.label(RichText::new("—").color(theme.text_muted));
                                } else if ui
                                    .add(egui::Link::new(
                                        RichText::new(&url).color(theme.clip_color).size(13.0),
                                    ))
                                    .on_hover_text("Open in browser")
                                    .clicked()
                                {
                                    let _ =
                                        std::process::Command::new("xdg-open").arg(&url).spawn();
                                }
                            });
                            cell(ui, widths[5], |ui| {
                                status_pill(ui, status.label, theme, status.kind, status.tooltip);
                            });
                            cell(ui, widths[6], |ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                let (toggle_label, toggle_fill, toggle_stroke, toggle_fg) =
                                    if tunnel.enabled {
                                        (
                                            "Off",
                                            theme.badge_bg,
                                            Stroke::new(1.0_f32, theme.card_stroke),
                                            theme.text_muted,
                                        )
                                    } else if theme.is_dark {
                                        (
                                            "On",
                                            Color32::from_rgb(20, 50, 32),
                                            Stroke::new(1.0_f32, Color32::from_rgb(34, 197, 94)),
                                            Color32::from_rgb(74, 222, 128),
                                        )
                                    } else {
                                        (
                                            "On",
                                            Color32::from_rgb(220, 252, 231),
                                            Stroke::new(1.0_f32, Color32::from_rgb(74, 222, 128)),
                                            Color32::from_rgb(21, 128, 61),
                                        )
                                    };
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(toggle_label)
                                                .color(toggle_fg)
                                                .family(bold_family())
                                                .size(12.5),
                                        )
                                        .fill(toggle_fill)
                                        .stroke(toggle_stroke)
                                        .corner_radius(5),
                                    )
                                    .clicked()
                                {
                                    action = Some(RowAction::Toggle(index));
                                }
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Edit")
                                                .color(theme.text_primary)
                                                .family(bold_family())
                                                .size(12.5),
                                        )
                                        .fill(theme.badge_bg)
                                        .stroke(Stroke::new(1.0_f32, theme.card_stroke))
                                        .corner_radius(5),
                                    )
                                    .clicked()
                                {
                                    action = Some(RowAction::Edit(index));
                                }
                                if ui.add(theme.danger_button("Remove")).clicked() {
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

fn direction_pill(ui: &mut egui::Ui, direction: Direction, theme: &ThemePalette) {
    let (text, bg, stroke_color, fg, tooltip) = match direction {
        Direction::Local => {
            if theme.is_dark {
                (
                    "Local",
                    Color32::from_rgb(14, 38, 54),
                    Color32::from_rgb(2, 132, 199),
                    Color32::from_rgb(56, 189, 248),
                    "Local forward (remote -> local)",
                )
            } else {
                (
                    "Local",
                    Color32::from_rgb(224, 242, 254),
                    Color32::from_rgb(56, 189, 248),
                    Color32::from_rgb(3, 105, 161),
                    "Local forward (remote -> local)",
                )
            }
        }
        Direction::Reverse => {
            if theme.is_dark {
                (
                    "Reverse",
                    Color32::from_rgb(38, 20, 52),
                    Color32::from_rgb(147, 51, 234),
                    Color32::from_rgb(216, 180, 254),
                    "Reverse forward (local -> remote)",
                )
            } else {
                (
                    "Reverse",
                    Color32::from_rgb(243, 232, 255),
                    Color32::from_rgb(192, 132, 252),
                    Color32::from_rgb(107, 33, 168),
                    "Reverse forward (local -> remote)",
                )
            }
        }
    };

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(64.0, 22.0), egui::Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(5),
        bg,
        Stroke::new(1.0_f32, stroke_color),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::new(FONT_PILL, bold_family()),
        fg,
    );
    resp.on_hover_text(tooltip);
}

fn header_row(ui: &mut egui::Ui, widths: &[f32; 7], theme: &ThemePalette) {
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), ROW_H));
    ui.painter()
        .rect_filled(rect, CornerRadius::same(6), theme.header_fill);
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
                        .family(bold_family())
                        .size(FONT_HEADER)
                        .color(theme.header_text),
                );
            });
        }
    });
}

fn paint_row_bg(ui: &mut egui::Ui, row_i: usize, theme: &ThemePalette) {
    let rect = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), ROW_H));
    if ui.rect_contains_pointer(rect) {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), theme.row_hover);
    } else if row_i % 2 == 1 {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), theme.row_stripe);
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    On,
    Warn,
    Error,
    Off,
}

struct TunnelStatus<'a> {
    label: &'static str,
    kind: StatusKind,
    tooltip: Option<&'a str>,
}

fn tunnel_status<'a>(tunnel: &'a crate::ports::Tunnel) -> TunnelStatus<'a> {
    if let Some(error) = &tunnel.error {
        return TunnelStatus {
            label: "failed",
            kind: StatusKind::Error,
            tooltip: Some(error.as_str()),
        };
    }
    if tunnel.enabled {
        TunnelStatus {
            label: "ON",
            kind: StatusKind::On,
            tooltip: None,
        }
    } else if tunnel.manual_off {
        TunnelStatus {
            label: "MANUAL OFF",
            kind: StatusKind::Off,
            tooltip: None,
        }
    } else if !tunnel.present {
        TunnelStatus {
            label: "TARGET DOWN",
            kind: StatusKind::Warn,
            tooltip: Some("Remote port is not currently listening"),
        }
    } else {
        TunnelStatus {
            label: "off",
            kind: StatusKind::Off,
            tooltip: None,
        }
    }
}

fn status_pill(
    ui: &mut egui::Ui,
    text: &str,
    theme: &ThemePalette,
    kind: StatusKind,
    tooltip: Option<&str>,
) {
    let (bg, stroke, fg) = match kind {
        StatusKind::On => {
            if theme.is_dark {
                (
                    Color32::from_rgb(18, 48, 28),
                    Stroke::new(1.0_f32, Color32::from_rgb(34, 197, 94)),
                    Color32::from_rgb(74, 222, 128),
                )
            } else {
                (
                    Color32::from_rgb(220, 252, 231),
                    Stroke::new(1.0_f32, Color32::from_rgb(74, 222, 128)),
                    Color32::from_rgb(21, 128, 61),
                )
            }
        }
        StatusKind::Error => {
            if theme.is_dark {
                (
                    Color32::from_rgb(56, 18, 22),
                    Stroke::new(1.0_f32, Color32::from_rgb(239, 68, 68)),
                    Color32::from_rgb(248, 113, 113),
                )
            } else {
                (
                    Color32::from_rgb(254, 226, 226),
                    Stroke::new(1.0_f32, Color32::from_rgb(248, 113, 113)),
                    Color32::from_rgb(185, 28, 28),
                )
            }
        }
        StatusKind::Warn => {
            if theme.is_dark {
                (
                    Color32::from_rgb(52, 38, 14),
                    Stroke::new(1.0_f32, Color32::from_rgb(234, 179, 8)),
                    Color32::from_rgb(250, 204, 21),
                )
            } else {
                (
                    Color32::from_rgb(254, 243, 199),
                    Stroke::new(1.0_f32, Color32::from_rgb(245, 158, 11)),
                    Color32::from_rgb(180, 83, 9),
                )
            }
        }
        StatusKind::Off => {
            if theme.is_dark {
                (
                    Color32::from_rgb(26, 30, 38),
                    Stroke::new(1.0_f32, Color32::from_rgb(51, 65, 85)),
                    Color32::from_rgb(148, 163, 184),
                )
            } else {
                (
                    Color32::from_rgb(241, 245, 249),
                    Stroke::new(1.0_f32, Color32::from_rgb(203, 213, 225)),
                    Color32::from_rgb(71, 85, 105),
                )
            }
        }
    };

    let approx_char_w = 7.5_f32;
    let pill_w = (text.chars().count() as f32 * approx_char_w + 16.0).max(46.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(pill_w, 22.0), egui::Sense::hover());
    ui.painter()
        .rect(rect, CornerRadius::same(5), bg, stroke, StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::new(FONT_PILL, bold_family()),
        fg,
    );
    if let Some(tip) = tooltip {
        resp.on_hover_text(tip);
    }
}
