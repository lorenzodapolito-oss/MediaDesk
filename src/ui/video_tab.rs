//! The Video tab: a batch converter (H.264 + AAC, max 1280 px wide) with audio track and
//! subtitle choice, colour correction, a manual or automatic crop on a preview frame, and a
//! per-file status.

use super::widgets::{notice_color, texture_from_bytes, Notice, Selection};
use super::Tools;
use crate::ffmpeg::{self, new_cancel, Cancel};
use crate::settings::Settings;
use crate::theme;
use crate::util::{clock, open_folder};
use crate::video::convert::{self, AudioChoice, Container, Options, PRESETS};
use crate::video::filters::Crop;
use crate::video::probe::{self, Probe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Waiting,
    Running(f32),
    Done(String),
    Failed(String),
    Cancelled,
}

struct Entry {
    path: PathBuf,
    size_mb: f64,
    probe: Option<Probe>,
    options: Options,
    status: Status,
}

enum Msg {
    Probed(PathBuf, Result<Probe, String>),
    Preview(PathBuf, Result<Vec<u8>, String>),
    Detected(PathBuf, Result<Crop, String>),
    Started(usize),
    Progress(usize, f32),
    Finished(usize, Status),
    AllDone,
}

struct Frame {
    path: PathBuf,
    texture: egui::TextureHandle,
}

pub struct VideoTab {
    in_dir: String,
    out_dir: String,
    entries: Vec<Entry>,
    selection: Selection,
    /// The options panel; its crop belongs to the first selected file.
    edit: Options,
    loaded: Option<usize>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    cancel: Cancel,
    running: bool,
    notice: Option<Notice>,
    frame: Option<Frame>,
    preview_busy: bool,
    ctx: egui::Context,
}

fn status_text(s: &Status) -> String {
    match s {
        Status::Waiting => "In attesa".into(),
        Status::Running(f) => format!("In corso {:.0}%", f * 100.0),
        Status::Done(n) => format!("✔ {n}"),
        Status::Failed(e) => format!("✖ {e}"),
        Status::Cancelled => "Annullato".into(),
    }
}

/// The options as a short line for the list.
fn summary(o: &Options) -> String {
    let mut parts = vec![
        o.container.ext().to_uppercase(),
        format!("CRF {}", o.crf),
        o.preset.clone(),
    ];
    if o.crop.is_some_and(|c| !c.is_none()) {
        parts.push("crop".into());
    }
    if o.color.enabled {
        parts.push("colore".into());
    }
    if o.hardsub.is_some() {
        parts.push("sub".into());
    }
    parts.join(" · ")
}

impl VideoTab {
    pub fn new(ctx: &egui::Context, settings: &Settings) -> Self {
        let (tx, rx) = channel();
        let mut tab = Self {
            in_dir: settings.video_in.clone(),
            out_dir: settings.video_out.clone(),
            entries: Vec::new(),
            selection: Selection::default(),
            edit: Options::default(),
            loaded: None,
            tx,
            rx,
            cancel: new_cancel(),
            running: false,
            notice: None,
            frame: None,
            preview_busy: false,
            ctx: ctx.clone(),
        };
        tab.selection.resize(0);
        tab
    }

    pub fn store_settings(&self, s: &mut Settings) {
        s.video_in = self.in_dir.clone();
        s.video_out = self.out_dir.clone();
    }

    pub fn busy(&self) -> bool {
        self.running || self.preview_busy
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some(Notice {
            text: text.into(),
            error,
        });
    }

    // ---- folder scan -------------------------------------------------------------------

    /// Lists the remembered folder again when the program starts.
    pub fn rescan_remembered(&mut self, tools: &Tools) {
        if !self.in_dir.trim().is_empty() && Path::new(self.in_dir.trim()).is_dir() {
            self.scan(tools);
        }
    }

    fn scan(&mut self, tools: &Tools) {
        if self.running {
            return;
        }
        self.entries.clear();
        self.selection.resize(0);
        self.loaded = None;
        self.frame = None;
        let files = match crate::video::list_folder(Path::new(self.in_dir.trim())) {
            Ok(f) => f,
            Err(e) => {
                self.say(format!("Cartella non leggibile: {e}"), true);
                return;
            }
        };
        let Some(ffprobe) = tools.ffprobe.clone() else {
            self.say(
                "ffprobe non trovato: non posso leggere tracce e durata (vedi Impostazioni).",
                true,
            );
            return;
        };
        for p in &files {
            let size_mb = std::fs::metadata(p)
                .map(|m| m.len() as f64 / 1_048_576.0)
                .unwrap_or(0.0);
            self.entries.push(Entry {
                path: p.clone(),
                size_mb,
                probe: None,
                options: Options::default(),
                status: Status::Waiting,
            });
        }
        self.selection.resize(self.entries.len());
        self.say(format!("{} video trovati", files.len()), false);
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            for p in files {
                let res = ffmpeg::capture(&ffprobe, &probe::args(&p), Duration::from_secs(30))
                    .and_then(|(out, _)| probe::parse(&out));
                if tx.send(Msg::Probed(p, res)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
    }

    fn first(&self) -> Option<usize> {
        self.selection.first().filter(|i| *i < self.entries.len())
    }

    /// Loads the panel from the first selected file when the selection moved.
    fn sync_selection(&mut self) {
        let first = self.first();
        if first != self.loaded {
            self.loaded = first;
            self.frame = None;
            if let Some(i) = first {
                self.edit = self.entries[i].options.clone();
            }
        }
    }

    /// Copies a changed panel to the selected files (the crop only to the first).
    fn apply_edit(&mut self) {
        let first = self.first();
        for i in self.selection.indices() {
            let Some(e) = self.entries.get_mut(i) else {
                continue;
            };
            let crop = e.options.crop;
            e.options = self.edit.clone();
            if Some(i) != first {
                e.options.crop = crop;
            }
        }
    }

    // ---- preview and crop --------------------------------------------------------------

    fn make_preview(&mut self, tools: &Tools) {
        let (Some(i), Some(ffmpeg_path)) = (self.first(), tools.ffmpeg.clone()) else {
            self.say("Seleziona un file (e serve ffmpeg).", true);
            return;
        };
        let path = self.entries[i].path.clone();
        let at = convert::sample_time(
            self.entries[i]
                .probe
                .as_ref()
                .map(|p| p.duration)
                .unwrap_or(0.0),
        );
        self.preview_busy = true;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let png =
                std::env::temp_dir().join(format!("mediadesk-preview-{}.png", std::process::id()));
            let res = ffmpeg::capture(
                &ffmpeg_path,
                &convert::preview_args(&path, at, &png),
                Duration::from_secs(60),
            )
            .and_then(|_| std::fs::read(&png).map_err(|e| e.to_string()));
            let _ = std::fs::remove_file(&png);
            let _ = tx.send(Msg::Preview(path, res));
            ctx.request_repaint();
        });
    }

    fn auto_crop(&mut self, tools: &Tools) {
        let (Some(i), Some(ffmpeg_path)) = (self.first(), tools.ffmpeg.clone()) else {
            self.say("Seleziona un file (e serve ffmpeg).", true);
            return;
        };
        let Some(pr) = self.entries[i].probe.clone() else {
            self.say("Il file non è ancora stato letto.", true);
            return;
        };
        let path = self.entries[i].path.clone();
        let at = convert::sample_time(pr.duration);
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let res = ffmpeg::capture(
                &ffmpeg_path,
                &convert::cropdetect_args(&path, at),
                Duration::from_secs(60),
            )
            .map(|(_, err)| match probe::parse_cropdetect(&err) {
                Some((w, h, x, y)) => Crop::from_detect(w, h, x, y, pr.width, pr.height),
                None => Crop::default(),
            });
            let _ = tx.send(Msg::Detected(path, res));
            ctx.request_repaint();
        });
    }

    // ---- conversion --------------------------------------------------------------------

    fn start(&mut self, tools: &Tools) {
        let Some(ffmpeg_path) = tools.ffmpeg.clone() else {
            self.say("ffmpeg non trovato (vedi Impostazioni).", true);
            return;
        };
        if self.entries.is_empty() {
            self.say("Nessun video: scegli una cartella e premi Scansiona.", true);
            return;
        }
        let out = PathBuf::from(self.out_dir.trim());
        if self.out_dir.trim().is_empty() {
            self.say("Scegli la cartella di destinazione.", true);
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&out) {
            self.say(
                format!("Cartella di destinazione non utilizzabile: {e}"),
                true,
            );
            return;
        }
        let jobs: Vec<(usize, PathBuf, Options, Option<Probe>)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.path.clone(), e.options.clone(), e.probe.clone()))
            .collect();
        for e in &mut self.entries {
            e.status = Status::Waiting;
        }
        self.cancel = new_cancel();
        self.running = true;
        let (tx, ctx, cancel) = (self.tx.clone(), self.ctx.clone(), self.cancel.clone());
        std::thread::spawn(move || {
            for (i, path, options, pr) in jobs {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send(Msg::Finished(i, Status::Cancelled));
                    continue;
                }
                let _ = tx.send(Msg::Started(i));
                let Some(pr) = pr else {
                    let _ = tx.send(Msg::Finished(
                        i,
                        Status::Failed("file non leggibile da ffprobe".into()),
                    ));
                    continue;
                };
                let target = convert::output_path(&path, &out, options.container);
                let args = convert::args(&path, &target, &options, &pr);
                let total = pr.duration.max(0.001);
                let progress_tx = tx.clone();
                let progress_ctx = ctx.clone();
                let result = ffmpeg::run(&ffmpeg_path, &args, &cancel, |secs| {
                    let _ =
                        progress_tx.send(Msg::Progress(i, (secs / total).clamp(0.0, 1.0) as f32));
                    progress_ctx.request_repaint();
                });
                let status = match result {
                    Ok(o) if o.cancelled => {
                        let _ = std::fs::remove_file(&target);
                        Status::Cancelled
                    }
                    Ok(o) if o.success => Status::Done(
                        target
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    ),
                    Ok(o) => {
                        let _ = std::fs::remove_file(&target);
                        Status::Failed(
                            o.stderr_tail
                                .lines()
                                .last()
                                .unwrap_or("errore ffmpeg")
                                .to_string(),
                        )
                    }
                    Err(e) => Status::Failed(e),
                };
                let _ = tx.send(Msg::Finished(i, status));
                ctx.request_repaint();
            }
            let _ = tx.send(Msg::AllDone);
            ctx.request_repaint();
        });
    }

    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Probed(p, res) => {
                    if let Some(e) = self.entries.iter_mut().find(|e| e.path == p) {
                        match res {
                            Ok(pr) => e.probe = Some(pr),
                            Err(err) => e.status = Status::Failed(format!("lettura: {err}")),
                        }
                    }
                }
                Msg::Preview(p, res) => {
                    self.preview_busy = false;
                    match res {
                        Ok(bytes) => {
                            if let Some(t) =
                                texture_from_bytes(&self.ctx, "video-preview", &bytes, 1280)
                            {
                                if self.first().is_some_and(|i| self.entries[i].path == p) {
                                    self.frame = Some(Frame {
                                        path: p,
                                        texture: t,
                                    });
                                }
                            }
                        }
                        Err(e) => self.say(format!("Anteprima non riuscita: {e}"), true),
                    }
                }
                Msg::Detected(p, res) => match res {
                    Ok(crop) => {
                        if let Some(e) = self.entries.iter_mut().find(|e| e.path == p) {
                            e.options.crop = (!crop.is_none()).then_some(crop);
                        }
                        if self.first().is_some_and(|i| self.entries[i].path == p) {
                            self.edit.crop = (!crop.is_none()).then_some(crop);
                        }
                        self.say(
                            if crop.is_none() {
                                "Nessuna banda nera trovata."
                            } else {
                                "Ritaglio automatico applicato."
                            },
                            false,
                        );
                    }
                    Err(e) => self.say(format!("Rilevamento non riuscito: {e}"), true),
                },
                Msg::Started(i) => {
                    if let Some(e) = self.entries.get_mut(i) {
                        e.status = Status::Running(0.0);
                    }
                }
                Msg::Progress(i, f) => {
                    if let Some(e) = self.entries.get_mut(i) {
                        e.status = Status::Running(f);
                    }
                }
                Msg::Finished(i, s) => {
                    if let Some(e) = self.entries.get_mut(i) {
                        e.status = s;
                    }
                }
                Msg::AllDone => {
                    self.running = false;
                    let done = self
                        .entries
                        .iter()
                        .filter(|e| matches!(e.status, Status::Done(_)))
                        .count();
                    let failed = self
                        .entries
                        .iter()
                        .filter(|e| matches!(e.status, Status::Failed(_)))
                        .count();
                    self.say(
                        format!("Conversione finita: {done} riusciti, {failed} con errori."),
                        failed > 0,
                    );
                }
            }
        }
    }

    // ---- drawing -----------------------------------------------------------------------

    pub fn ui(&mut self, ui: &mut egui::Ui, tools: &Tools) {
        self.poll();
        self.sync_selection();
        let mut scan = false;
        let mut start = false;
        let mut stop = false;
        let mut preview = false;
        let mut auto = false;

        egui::Panel::top("video_top")
            .frame(egui::Frame::new().fill(theme::BG_LIGHT).inner_margin(8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if theme::button(ui, "Cartella video", theme::PRIMARY).clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Cartella con i video")
                            .pick_folder()
                        {
                            self.in_dir = p.to_string_lossy().into_owned();
                            scan = true;
                        }
                    }
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.in_dir)
                            .desired_width(ui.available_width() - 330.0),
                    );
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        scan = true;
                    }
                    if theme::button(ui, "🔍 Scansiona", theme::PRIMARY).clicked() {
                        scan = true;
                    }
                });
                ui.horizontal(|ui| {
                    if theme::button(ui, "Cartella di uscita", theme::PRIMARY).clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_title("Dove salvare i video convertiti")
                            .pick_folder()
                        {
                            self.out_dir = p.to_string_lossy().into_owned();
                        }
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.out_dir)
                            .desired_width(ui.available_width() - 330.0),
                    );
                    if theme::button(ui, "📁 Apri", theme::PRIMARY).clicked()
                        && !self.out_dir.trim().is_empty()
                    {
                        if let Err(e) = open_folder(Path::new(self.out_dir.trim())) {
                            self.say(e, true);
                        }
                    }
                    if self.running {
                        if theme::button(ui, "⏹ Ferma", theme::ACCENT).clicked() {
                            stop = true;
                        }
                    } else if theme::button(ui, "▶ Avvia conversione", theme::TEAL).clicked() {
                        start = true;
                    }
                });
            });

        egui::Panel::bottom("video_status")
            .frame(egui::Frame::new().fill(theme::BG_LIGHT).inner_margin(6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.running {
                        let done = self
                            .entries
                            .iter()
                            .filter(|e| {
                                matches!(
                                    e.status,
                                    Status::Done(_) | Status::Failed(_) | Status::Cancelled
                                )
                            })
                            .count();
                        ui.label(format!(
                            "Conversione {}/{}",
                            (done + 1).min(self.entries.len()),
                            self.entries.len()
                        ));
                    } else if let Some(n) = &self.notice {
                        ui.colored_label(notice_color(n), &n.text);
                    }
                });
            });

        egui::Panel::right("video_right")
            .default_size(440.0)
            .frame(egui::Frame::new().fill(theme::BG_DARK).inner_margin(8))
            .show(ui, |ui| {
                self.preview_panel(ui, &mut preview, &mut auto);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG_DARK).inner_margin(6))
            .show(ui, |ui| {
                let before = self.edit.clone();
                egui::Frame::new()
                    .fill(theme::BG_LIGHT)
                    .corner_radius(10.0)
                    .inner_margin(8)
                    .show(ui, |ui| self.options_panel(ui));
                if self.edit != before {
                    self.apply_edit();
                }
                ui.add_space(6.0);
                self.file_list(ui);
            });

        if scan {
            self.scan(tools);
        }
        if start {
            self.start(tools);
        }
        if stop {
            self.cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if preview {
            self.make_preview(tools);
        }
        if auto {
            self.auto_crop(tools);
        }
    }

    fn options_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("Opzioni di codifica")
                .strong()
                .size(15.0),
        );
        ui.horizontal(|ui| {
            ui.label("Contenitore:");
            ui.selectable_value(&mut self.edit.container, Container::Mkv, "MKV");
            ui.selectable_value(&mut self.edit.container, Container::Mp4, "MP4");
            ui.add_space(12.0);
            ui.label("Qualità (CRF 15–45):");
            ui.add(egui::Slider::new(&mut self.edit.crf, 15..=45));
        });
        ui.horizontal(|ui| {
            ui.label("Preset:");
            egui::ComboBox::from_id_salt("video_preset")
                .selected_text(self.edit.preset.clone())
                .show_ui(ui, |ui| {
                    for p in PRESETS {
                        ui.selectable_value(&mut self.edit.preset, p.to_string(), p);
                    }
                });
        });
        let probe = self.first().and_then(|i| self.entries[i].probe.clone());
        ui.horizontal(|ui| {
            ui.label("Traccia audio:");
            let text = match (&self.edit.audio, &probe) {
                (AudioChoice::Track(n), Some(p)) => p
                    .audio
                    .iter()
                    .find(|a| a.order == *n)
                    .map(|a| a.label())
                    .unwrap_or_else(|| format!("Traccia {n}")),
                _ => "Tutte le tracce".into(),
            };
            egui::ComboBox::from_id_salt("video_audio")
                .width(300.0)
                .selected_text(text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.edit.audio, AudioChoice::All, "Tutte le tracce");
                    if let Some(p) = &probe {
                        for a in &p.audio {
                            ui.selectable_value(
                                &mut self.edit.audio,
                                AudioChoice::Track(a.order),
                                a.label(),
                            );
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Sottotitoli nel video:");
            let text = match (&self.edit.hardsub, &probe) {
                (Some(n), Some(p)) => p
                    .subs
                    .iter()
                    .find(|s| s.order == *n)
                    .map(|s| s.label())
                    .unwrap_or_else(|| format!("Sottotitolo {n}")),
                _ => "Nessuno (mantieni i sottotitoli del file)".into(),
            };
            egui::ComboBox::from_id_salt("video_subs")
                .width(300.0)
                .selected_text(text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.edit.hardsub,
                        None,
                        "Nessuno (mantieni i sottotitoli del file)",
                    );
                    if let Some(p) = &probe {
                        for s in p.subs.iter().filter(|s| s.is_text) {
                            ui.selectable_value(&mut self.edit.hardsub, Some(s.order), s.label());
                        }
                    }
                });
        });
        ui.separator();
        ui.label(
            egui::RichText::new("Correzione colore e bilanciamento del bianco")
                .strong()
                .size(15.0),
        );
        let c = &mut self.edit.color;
        ui.checkbox(&mut c.enabled, "Attiva i filtri colore");
        ui.add_enabled_ui(c.enabled, |ui| {
            egui::Grid::new("video_color")
                .num_columns(4)
                .show(ui, |ui| {
                    ui.label("Luminosità");
                    ui.add(egui::Slider::new(&mut c.brightness, -1.0..=1.0));
                    ui.label("Contrasto");
                    ui.add(egui::Slider::new(&mut c.contrast, 0.0..=2.0));
                    ui.end_row();
                    ui.label("Saturazione");
                    ui.add(egui::Slider::new(&mut c.saturation, 0.0..=3.0));
                    ui.label("Gamma");
                    ui.add(egui::Slider::new(&mut c.gamma, 0.1..=3.0));
                    ui.end_row();
                    ui.label("Temperatura");
                    ui.add(egui::Slider::new(&mut c.temperature, -100..=100));
                    ui.label("Tinta (°)");
                    ui.add(egui::Slider::new(&mut c.hue, -180.0..=180.0));
                    ui.end_row();
                });
            if ui.button("Azzera colori").clicked() {
                let enabled = c.enabled;
                *c = crate::video::filters::Color {
                    enabled,
                    ..Default::default()
                };
            }
        });
    }

    fn file_list(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("File trovati").strong().size(14.0));
        let mut click: Option<(usize, bool, bool)> = None;
        egui::Frame::new()
            .fill(theme::LIST_BG)
            .corner_radius(10.0)
            .inner_margin(4)
            .show(ui, |ui| {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.entries.is_empty() {
                            ui.label(
                                egui::RichText::new(
                                    "Scegli la cartella dei video e premi Scansiona.",
                                )
                                .color(theme::MUTED),
                            );
                        }
                        for (i, e) in self.entries.iter().enumerate() {
                            let selected = self.selection.contains(i);
                            let name = e
                                .path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            let dur = e
                                .probe
                                .as_ref()
                                .map(|p| clock(p.duration))
                                .unwrap_or_else(|| "…".into());
                            let line = format!(
                                "{name}   ({:.0} MB, {dur})   {}   —   {}",
                                e.size_mb,
                                summary(&e.options),
                                status_text(&e.status)
                            );
                            let resp = ui.selectable_label(selected, line);
                            if resp.clicked() {
                                let m = ui.input(|i| i.modifiers);
                                click = Some((i, m.command, m.shift));
                            }
                            if let Status::Running(f) = e.status {
                                ui.add(
                                    egui::ProgressBar::new(f)
                                        .desired_height(5.0)
                                        .fill(theme::TEAL),
                                );
                            }
                        }
                    });
            });
        if let Some((i, ctrl, shift)) = click {
            self.selection.click(i, ctrl, shift);
        }
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui, preview: &mut bool, auto: &mut bool) {
        ui.label(
            egui::RichText::new("Anteprima e ritaglio")
                .strong()
                .size(15.0),
        );
        let first = self.first();
        let src = first.and_then(|i| self.entries[i].probe.as_ref().map(|p| (p.width, p.height)));
        ui.horizontal(|ui| {
            if theme::button(ui, "📷 Anteprima", theme::PRIMARY).clicked() {
                *preview = true;
            }
            if theme::button(ui, "🎯 Auto", theme::PRIMARY).clicked() {
                *auto = true;
            }
            if theme::button(ui, "🔄 Azzera", theme::PRIMARY).clicked() {
                self.edit.crop = None;
            }
        });
        let mut crop = self.edit.crop.unwrap_or_default();
        if let Some((w, h)) = src {
            let (w, h) = (w.max(16), h.max(16));
            ui.label(format!(
                "Fotogramma originale {w}×{h}; ritaglio in pixel per lato"
            ));
            let mut changed = false;
            egui::Grid::new("video_crop").num_columns(2).show(ui, |ui| {
                changed |= ui
                    .add(egui::Slider::new(&mut crop.left, 0..=w / 2).text("sinistra"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut crop.right, 0..=w / 2).text("destra"))
                    .changed();
                ui.end_row();
                changed |= ui
                    .add(egui::Slider::new(&mut crop.top, 0..=h / 2).text("sopra"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut crop.bottom, 0..=h / 2).text("sotto"))
                    .changed();
                ui.end_row();
            });
            if changed {
                let c = crop.clamped(w, h);
                self.edit.crop = (!c.is_none()).then_some(c);
            }
        } else {
            ui.label(egui::RichText::new("Seleziona un file per ritagliarlo.").color(theme::MUTED));
        }
        let crop = self.edit.crop.unwrap_or_default();
        egui::Frame::new()
            .fill(theme::BG_LIGHT)
            .corner_radius(10.0)
            .inner_margin(6)
            .show(ui, |ui| {
                let width = ui.available_width();
                match (&self.frame, src) {
                    (Some(f), Some((w, h)))
                        if self.first().is_some_and(|i| self.entries[i].path == f.path) =>
                    {
                        let size = f.texture.size_vec2();
                        let shown = egui::vec2(width, width * size.y / size.x.max(1.0));
                        let (rect, _) = ui.allocate_exact_size(shown, egui::Sense::hover());
                        ui.painter().image(
                            f.texture.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                        let (sx, sy) = (rect.width() / w as f32, rect.height() / h as f32);
                        let inner = egui::Rect::from_min_max(
                            egui::pos2(
                                rect.left() + crop.left as f32 * sx,
                                rect.top() + crop.top as f32 * sy,
                            ),
                            egui::pos2(
                                rect.right() - crop.right as f32 * sx,
                                rect.bottom() - crop.bottom as f32 * sy,
                            ),
                        );
                        let shade = egui::Color32::from_black_alpha(150);
                        let p = ui.painter();
                        p.rect_filled(
                            egui::Rect::from_min_max(
                                rect.min,
                                egui::pos2(rect.right(), inner.top()),
                            ),
                            0.0,
                            shade,
                        );
                        p.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(rect.left(), inner.bottom()),
                                rect.max,
                            ),
                            0.0,
                            shade,
                        );
                        p.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(rect.left(), inner.top()),
                                egui::pos2(inner.left(), inner.bottom()),
                            ),
                            0.0,
                            shade,
                        );
                        p.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(inner.right(), inner.top()),
                                egui::pos2(rect.right(), inner.bottom()),
                            ),
                            0.0,
                            shade,
                        );
                        p.rect_stroke(
                            inner,
                            0.0,
                            egui::Stroke::new(2.0, theme::TEAL),
                            egui::StrokeKind::Inside,
                        );
                    }
                    _ => {
                        ui.add_space(30.0);
                        let text = if self.preview_busy {
                            "Creo l'anteprima…"
                        } else {
                            "Premi «Anteprima» per vedere un fotogramma"
                        };
                        ui.vertical_centered(|ui| {
                            ui.label(egui::RichText::new(text).color(theme::MUTED))
                        });
                        ui.add_space(30.0);
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_names_what_will_change() {
        let mut o = Options::default();
        assert_eq!(summary(&o), "MKV · CRF 20 · medium");
        o.color.enabled = true;
        o.crop = Some(Crop {
            left: 4,
            right: 4,
            top: 0,
            bottom: 0,
        });
        o.hardsub = Some(0);
        assert!(summary(&o).ends_with("crop · colore · sub"));
    }

    #[test]
    fn status_lines_read_well() {
        assert_eq!(status_text(&Status::Running(0.5)), "In corso 50%");
        assert!(status_text(&Status::Failed("x".into())).starts_with('✖'));
    }
}
