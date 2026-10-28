//! The window: a tab bar (Audio | Video) over the two tabs, plus the settings window.

mod audio_tab;
mod video_tab;
pub mod widgets;

use crate::settings::Settings;
use crate::theme;
use std::path::PathBuf;

pub const APP_TITLE: &str = "MediaDesk di Lorenzo D'Apolito";

/// The external tools both tabs drive.
#[derive(Clone, Debug, Default)]
pub struct Tools {
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
}

impl Tools {
    pub fn find(configured: &str) -> Self {
        Self {
            ffmpeg: crate::ffmpeg::find_tool("ffmpeg", configured),
            ffprobe: crate::ffmpeg::find_tool("ffprobe", configured),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Audio,
    Video,
}

pub struct App {
    tab: Tab,
    settings: Settings,
    tools: Tools,
    audio: audio_tab::AudioTab,
    video: video_tab::VideoTab,
    show_settings: bool,
}

impl App {
    pub fn new(ctx: &egui::Context) -> Self {
        theme::apply(ctx);
        let settings = Settings::load();
        let tools = Tools::find(&settings.ffmpeg_path);
        let audio = audio_tab::AudioTab::new(ctx, &settings);
        let mut video = video_tab::VideoTab::new(ctx, &settings);
        video.rescan_remembered(&tools);
        let tab = if settings.tab == "video" {
            Tab::Video
        } else {
            Tab::Audio
        };
        Self {
            tab,
            settings,
            tools,
            audio,
            video,
            show_settings: false,
        }
    }

    fn save_settings(&mut self) {
        self.settings.tab = if self.tab == Tab::Video {
            "video"
        } else {
            "audio"
        }
        .into();
        self.audio.store_settings(&mut self.settings);
        self.video.store_settings(&mut self.settings);
        let _ = self.settings.save();
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        let mut apply = false;
        egui::Window::new("Impostazioni")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    "Cartella o file di ffmpeg (vuoto = cerca accanto al programma e nel PATH)",
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.ffmpeg_path).desired_width(420.0),
                );
                ui.horizontal(|ui| {
                    if ui.button("Sfoglia…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Cartella di ffmpeg")
                            .pick_folder()
                        {
                            self.settings.ffmpeg_path = p.to_string_lossy().into_owned();
                        }
                    }
                    if ui.button("Cerca di nuovo").clicked() {
                        apply = true;
                    }
                });
                let status = |p: &Option<PathBuf>| {
                    p.as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "non trovato".into())
                };
                ui.label(format!("ffmpeg: {}", status(&self.tools.ffmpeg)));
                ui.label(format!("ffprobe: {}", status(&self.tools.ffprobe)));
                ui.separator();
                ui.label("Identificativo inviato a MusicBrainz");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.musicbrainz_agent)
                        .desired_width(420.0),
                );
                ui.horizontal(|ui| {
                    ui.label("Punteggio minimo MusicBrainz");
                    ui.add(egui::Slider::new(&mut self.settings.mb_min_score, 30..=100));
                });
            });
        if apply {
            self.tools = Tools::find(&self.settings.ffmpeg_path);
            let _ = self.settings.save();
        }
        if !open {
            self.show_settings = false;
            let _ = self.settings.save();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("tabs")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG_LIGHT)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in [(Tab::Audio, "🎧 Audio"), (Tab::Video, "🎬 Video")] {
                        let selected = self.tab == tab;
                        let text = egui::RichText::new(label).size(15.0).strong();
                        if ui
                            .add(
                                egui::Button::new(text)
                                    .selected(selected)
                                    .min_size(egui::vec2(110.0, 28.0)),
                            )
                            .clicked()
                        {
                            self.tab = tab;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("⚙ Impostazioni").clicked() {
                            self.show_settings = true;
                        }
                        if self.tools.ffmpeg.is_none() {
                            ui.colored_label(
                                theme::ACCENT,
                                "ffmpeg non trovato: conversione non disponibile",
                            );
                        }
                    });
                });
            });
        match self.tab {
            Tab::Audio => self.audio.ui(ui, &self.tools, &self.settings),
            Tab::Video => self.video.ui(ui, &self.tools),
        }
        self.settings_window(&ctx);
        // keep playback position and job progress moving
        if self.audio.busy() || self.video.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_settings();
    }
}
