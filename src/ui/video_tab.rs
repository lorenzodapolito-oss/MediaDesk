//! The Video tab: a batch converter (H.264 + AAC, max 1280 px wide) with audio track and
//! subtitle choice, colour correction, a manual or automatic crop on a preview frame, and a
//! per-file status.

use super::widgets::{notice_color, texture_from_bytes, Notice, Selection};
use super::Tools;
use crate::ffmpeg::{self, new_cancel, Cancel};
use crate::settings::Settings;
use crate::theme;
use crate::util::{clock, open_folder};
use crate::video::aspect;
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
    /// Where in the video (seconds) the preview and the crop analysis look; minute 10 at first.
    at: f64,
}

enum Msg {
    Probed(PathBuf, Result<Probe, String>),
    Preview(PathBuf, Result<Vec<u8>, String>),
    /// The 16:9 crop found at the analysis point; the flag says whether to tell the user.
    Detected(PathBuf, Result<Crop, String>, bool),
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
    /// A preview of the selected file is wanted as soon as its streams are known.
    want_preview: bool,
    /// Options edited in the panel go to every file (otherwise only to the selected ones).
    apply_all: bool,
    /// The frame being encoded right now, and the file it belongs to.
    live: Option<Frame>,
    live_name: String,
    live_stamp: Option<std::time::SystemTime>,
    live_checked: std::time::Instant,
    ctx: egui::Context,
}

/// The 16:9 crop of what `cropdetect` finds at `at` seconds (the whole frame's when it finds nothing).
fn detect_crop(ff: &Path, file: &Path, at: f64, pr: &Probe) -> Result<Crop, String> {
    let (_, err) = ffmpeg::capture(
        ff,
        &convert::cropdetect_args(file, at),
        Duration::from_secs(60),
    )?;
    Ok(match probe::parse_cropdetect(&err) {
        Some((w, h, x, y)) => aspect::from_detect(pr.width, pr.height, w, h, x, y),
        None => aspect::fit_frame(pr.width, pr.height),
    })
}

/// Where ffmpeg writes the frame it is encoding.
fn live_path() -> PathBuf {
    std::env::temp_dir().join(format!("mediadesk-live-{}.jpg", std::process::id()))
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
            want_preview: false,
            apply_all: true,
            live: None,
            live_name: String::new(),
            live_stamp: None,
            live_checked: std::time::Instant::now(),
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
        if self.in_dir.trim().is_empty() {
            self.say("Scegli prima la cartella dei video.", true);
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
        let (ffprobe, ffmpeg_path) = (tools.ffprobe.clone(), tools.ffmpeg.clone());
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
                at: 0.0,
            });
        }
        self.selection.resize(self.entries.len());
        if !self.entries.is_empty() {
            // the first file is selected, so its frame at minute 10 shows up straight away
            self.selection.only(0);
        }
        let found = if files.is_empty() {
            "Nessun video in questa cartella.".to_string()
        } else if tools.ffmpeg.is_none() && tools.ffprobe.is_none() {
            format!(
                "{} video trovati, ma ffmpeg non è stato trovato: vedi Impostazioni.",
                files.len()
            )
        } else {
            format!("{} video trovati", files.len())
        };
        self.say(
            found,
            files.is_empty() || (tools.ffmpeg.is_none() && tools.ffprobe.is_none()),
        );
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            for p in files {
                let res = probe::read(ffprobe.as_deref(), ffmpeg_path.as_deref(), &p);
                let found = res.as_ref().ok().cloned();
                if tx.send(Msg::Probed(p.clone(), res)).is_err() {
                    break;
                }
                ctx.request_repaint();
                // the 16:9 crop is found automatically, looking at minute 10 (or the middle)
                if let (Some(pr), Some(ff)) = (found, ffmpeg_path.as_deref()) {
                    let at = convert::sample_time(pr.duration);
                    let crop = detect_crop(ff, &p, at, &pr);
                    if tx.send(Msg::Detected(p, crop, false)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            }
        });
    }

    /// Shows the output folder in the file manager (created first when it does not exist yet).
    fn open_output(&mut self) {
        let dir = self.out_dir.trim().to_string();
        if dir.is_empty() {
            self.say("Scegli prima la cartella di uscita.", true);
            return;
        }
        let path = PathBuf::from(&dir);
        if let Err(e) = std::fs::create_dir_all(&path) {
            self.say(format!("Cartella non utilizzabile: {e}"), true);
            return;
        }
        if let Err(e) = open_folder(&path) {
            self.say(e, true);
        }
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
                self.want_preview = true;
            }
        }
    }

    /// Copies a changed panel to every file (or only the selected ones when "Applica a tutti"
    /// is off). The crop is different for each file: it goes only to the first selected one.
    fn apply_edit(&mut self) {
        let first = self.first();
        let targets: Vec<usize> = if self.apply_all {
            (0..self.entries.len()).collect()
        } else {
            self.selection.indices()
        };
        for i in targets {
            let Some(e) = self.entries.get_mut(i) else {
                continue;
            };
            let (crop, at) = (e.options.crop, e.at);
            e.options = self.edit.clone();
            e.at = at;
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
        let at = self.entries[i].at;
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

    /// Looks for the black bars at the point chosen with the slider and applies a 16:9 crop.
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
        let at = self.entries[i].at;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let res = detect_crop(&ffmpeg_path, &path, at, &pr);
            let _ = tx.send(Msg::Detected(path, res, true));
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
        self.live = None;
        self.live_stamp = None;
        let live = live_path();
        let _ = std::fs::remove_file(&live);
        let (tx, ctx, cancel) = (self.tx.clone(), self.ctx.clone(), self.cancel.clone());
        std::thread::spawn(move || {
            for (i, path, options, pr) in jobs {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send(Msg::Finished(i, Status::Cancelled));
                    continue;
                }
                let _ = tx.send(Msg::Started(i));
                let _ = std::fs::remove_file(&live);
                let Some(pr) = pr else {
                    let _ = tx.send(Msg::Finished(
                        i,
                        Status::Failed("file non leggibile da ffprobe".into()),
                    ));
                    continue;
                };
                let target = convert::output_path(&path, &out, options.container);
                let args = convert::args_with_live(&path, &target, &options, &pr, Some(&live));
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
                            Ok(pr) => {
                                e.at = convert::sample_time(pr.duration);
                                e.options.crop = Some(aspect::fit_frame(pr.width, pr.height));
                                e.probe = Some(pr);
                            }
                            Err(err) => e.status = Status::Failed(format!("lettura: {err}")),
                        }
                    }
                    if self.first().is_some_and(|i| self.entries[i].path == p) {
                        // the selected file's panel follows its fresh values
                        self.edit.crop = self.entries[self.first().unwrap_or(0)].options.crop;
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
                Msg::Detected(p, res, announce) => match res {
                    Ok(crop) => {
                        let crop = Some(crop);
                        if let Some(e) = self.entries.iter_mut().find(|e| e.path == p) {
                            e.options.crop = crop;
                        }
                        if self.first().is_some_and(|i| self.entries[i].path == p) {
                            self.edit.crop = crop;
                        }
                        if announce {
                            self.say("Ritaglio 16:9 applicato.", false);
                        }
                    }
                    Err(e) => {
                        if announce {
                            self.say(format!("Rilevamento non riuscito: {e}"), true);
                        }
                    }
                },
                Msg::Started(i) => {
                    if let Some(e) = self.entries.get_mut(i) {
                        e.status = Status::Running(0.0);
                        self.live_name = e
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        self.live = None;
                        self.live_stamp = None;
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
                    self.live = None;
                    let _ = std::fs::remove_file(live_path());
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

    /// Shows the newest frame ffmpeg wrote while converting (checked twice a second).
    fn refresh_live(&mut self) {
        if !self.running || self.live_checked.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.live_checked = std::time::Instant::now();
        let path = live_path();
        let Ok(meta) = std::fs::metadata(&path) else {
            return;
        };
        let stamp = meta.modified().ok();
        if stamp.is_some() && stamp == self.live_stamp {
            return;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        if let Some(texture) = texture_from_bytes(&self.ctx, "video-live", &bytes, 640) {
            self.live = Some(Frame { path, texture });
            self.live_stamp = stamp;
        }
    }

    // ---- drawing -----------------------------------------------------------------------

    pub fn ui(&mut self, ui: &mut egui::Ui, tools: &Tools) {
        self.poll();
        self.sync_selection();
        self.refresh_live();
        let mut scan = false;
        let mut start = false;
        let mut stop = false;
        let mut preview = false;
        let mut auto = false;
        let mut open = false;
        let before_edit = self.edit.clone();

        egui::Panel::top("video_top")
            .frame(theme::page(10).inner_margin(egui::Margin {
                left: 10,
                right: 10,
                top: 10,
                bottom: 0,
            }))
            .show(ui, |ui| {
                theme::card()
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
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
                            if theme::button(ui, "📁 Apri", theme::PRIMARY).clicked() {
                                open = true;
                            }
                            if self.running {
                                if theme::button(ui, "⏹ Ferma", theme::ACCENT).clicked() {
                                    stop = true;
                                }
                            } else if theme::button(ui, "▶ Avvia conversione", theme::TEAL)
                                .clicked()
                            {
                                start = true;
                            }
                        });
                    });
            });

        egui::Panel::bottom("video_status")
            .frame(theme::page(10).inner_margin(egui::Margin::symmetric(16, 6)))
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
            .default_size(470.0)
            .frame(theme::page(10))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    theme::card().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.preview_panel(ui, &mut preview, &mut auto);
                    });
                    ui.add_space(10.0);
                    theme::card().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.color_panel(ui);
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(theme::page(10))
            .show(ui, |ui| {
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.options_panel(ui)
                });
                if self.edit != before_edit {
                    self.apply_edit();
                }
                ui.add_space(10.0);
                self.file_list(ui);
            });

        if open {
            self.open_output();
        }
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
        if self.want_preview {
            match self.first() {
                Some(i) if self.entries[i].probe.is_some() => {
                    self.want_preview = false;
                    preview = true;
                }
                Some(_) => {}
                None => self.want_preview = false,
            }
        }
        if auto {
            self.auto_crop(tools);
            preview = true;
        }
        if preview && !self.running {
            self.make_preview(tools);
        }
    }

    fn options_panel(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Opzioni di codifica");
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
        ui.checkbox(
            &mut self.apply_all,
            "Applica le opzioni a tutti i file (altrimenti solo a quelli selezionati)",
        );
        ui.label(
            egui::RichText::new(
                "Uscita sempre 1280×720, H.264 livello 3.1, audio AAC 128 kbit/s stereo.",
            )
            .small()
            .color(theme::MUTED),
        );
    }

    fn color_panel(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Correzione colore");
        let c = &mut self.edit.color;
        ui.checkbox(&mut c.enabled, "Attiva i filtri colore");
        ui.add_enabled_ui(c.enabled, |ui| {
            ui.spacing_mut().slider_width = 170.0;
            egui::Grid::new("video_color")
                .num_columns(2)
                .spacing([14.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Luminosità");
                    ui.add(egui::Slider::new(&mut c.brightness, -1.0..=1.0));
                    ui.end_row();
                    ui.label("Contrasto");
                    ui.add(egui::Slider::new(&mut c.contrast, 0.0..=2.0));
                    ui.end_row();
                    ui.label("Saturazione");
                    ui.add(egui::Slider::new(&mut c.saturation, 0.0..=3.0));
                    ui.end_row();
                    ui.label("Gamma");
                    ui.add(egui::Slider::new(&mut c.gamma, 0.1..=3.0));
                    ui.end_row();
                    ui.label("Temperatura");
                    ui.add(egui::Slider::new(&mut c.temperature, -100..=100));
                    ui.end_row();
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
        let mut click: Option<(usize, bool, bool)> = None;
        theme::list_card().show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(format!("FILE TROVATI ({})", self.entries.len()))
                        .small()
                        .strong()
                        .color(theme::MUTED),
                );
            });
            ui.add_space(2.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.entries.is_empty() {
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new(
                                    "Scegli la cartella dei video e premi Scansiona.",
                                )
                                .color(theme::MUTED),
                            );
                        });
                    }
                    let row_h = 48.0;
                    for (i, e) in self.entries.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), row_h),
                            egui::Sense::click(),
                        );
                        let bg = rect.shrink2(egui::vec2(2.0, 1.0));
                        if self.selection.contains(i) {
                            ui.painter().rect_filled(bg, 10.0, theme::TEAL);
                        } else if resp.hovered() {
                            ui.painter()
                                .rect_filled(bg, 10.0, theme::lighten(theme::LIST_BG, 0.1));
                        } else if i % 2 == 1 {
                            ui.painter()
                                .rect_filled(bg, 10.0, theme::darken(theme::LIST_BG, 0.08));
                        }
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
                        let tracks = match (&e.options.audio, &e.probe) {
                            (AudioChoice::Track(_), _) => 1,
                            (AudioChoice::All, Some(p)) => p.audio.len(),
                            (AudioChoice::All, None) => 1,
                        };
                        let estimate = e
                            .probe
                            .as_ref()
                            .map(|p| convert::estimated_bytes(p.duration, e.options.crf, tracks))
                            .filter(|b| *b > 0)
                            .map(|b| format!("circa {}", convert::human_size(b)))
                            .unwrap_or_else(|| "…".into());
                        let detail = format!(
                            "{:.0} MB · uscita {estimate} · {dur} · {}",
                            e.size_mb,
                            summary(&e.options)
                        );
                        let left = rect.left() + 14.0;
                        ui.painter().text(
                            egui::pos2(left, rect.top() + 15.0),
                            egui::Align2::LEFT_CENTER,
                            name,
                            egui::FontId::proportional(14.5),
                            egui::Color32::WHITE,
                        );
                        ui.painter().text(
                            egui::pos2(left, rect.top() + 33.0),
                            egui::Align2::LEFT_CENTER,
                            detail,
                            egui::FontId::proportional(12.5),
                            egui::Color32::WHITE,
                        );
                        let status_color = match e.status {
                            Status::Done(_) => theme::TEAL,
                            Status::Failed(_) => theme::ACCENT,
                            _ => egui::Color32::WHITE,
                        };
                        ui.painter().text(
                            egui::pos2(rect.right() - 14.0, rect.top() + 24.0),
                            egui::Align2::RIGHT_CENTER,
                            status_text(&e.status),
                            egui::FontId::proportional(13.0),
                            status_color,
                        );
                        if let Status::Running(f) = e.status {
                            let bar = egui::Rect::from_min_size(
                                egui::pos2(bg.left() + 10.0, bg.bottom() - 6.0),
                                egui::vec2((bg.width() - 20.0) * f, 3.0),
                            );
                            ui.painter().rect_filled(bar, 2.0, theme::TEAL);
                        }
                        if resp.clicked() {
                            let m = ui.input(|i| i.modifiers);
                            click = Some((i, m.command, m.shift));
                        }
                    }
                });
        });
        if let Some((i, ctrl, shift)) = click {
            self.selection.click(i, ctrl, shift);
        }
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui, preview: &mut bool, auto: &mut bool) {
        theme::section(ui, "Anteprima e ritaglio 16:9");
        let first = self.first();
        let src = first.and_then(|i| self.entries[i].probe.as_ref().map(|p| (p.width, p.height)));
        ui.horizontal(|ui| {
            if theme::button(ui, "📷 Anteprima", theme::PRIMARY).clicked() {
                *preview = true;
            }
            if theme::button(ui, "🎯 Auto 16:9", theme::PRIMARY).clicked() {
                *auto = true;
            }
            if theme::button(ui, "🔄 Azzera", theme::PRIMARY).clicked() {
                if let Some((w, h)) = src {
                    self.edit.crop = Some(aspect::fit_frame(w, h));
                }
            }
        });
        // where in the video the preview and the automatic analysis look
        if let Some(i) = first {
            if let Some(duration) = self.entries[i].probe.as_ref().map(|p| p.duration) {
                let dur = duration.max(1.0);
                let mut at = self.entries[i].at.clamp(0.0, dur);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(clock(at)).strong());
                    ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(80.0);
                    let r = ui.add(egui::Slider::new(&mut at, 0.0..=dur).show_value(false));
                    ui.label(egui::RichText::new(clock(dur)).small().color(theme::MUTED));
                    if r.changed() {
                        self.entries[i].at = at;
                    }
                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                        *preview = true;
                    }
                });
                ui.label(
                    egui::RichText::new("Scegli il punto del video, poi premi «Auto 16:9».")
                        .small()
                        .color(theme::MUTED),
                );
            }
        }
        if let Some((w, h)) = src {
            let (w, h) = (w.max(32), h.max(18));
            let old = self.edit.crop.unwrap_or_else(|| aspect::fit_frame(w, h));
            let mut new = old;
            let (cw, ch) = aspect::size(w, h, old);
            ui.label(format!(
                "Fotogramma {w}×{h} · finestra {cw}×{ch} · uscita 1280×720"
            ));
            ui.spacing_mut().slider_width = 110.0;
            egui::Grid::new("video_crop").num_columns(2).show(ui, |ui| {
                ui.add(egui::Slider::new(&mut new.left, 0..=w / 2).text("sinistra"));
                ui.add(egui::Slider::new(&mut new.right, 0..=w / 2).text("destra"));
                ui.end_row();
                ui.add(egui::Slider::new(&mut new.top, 0..=h / 2).text("sopra"));
                ui.add(egui::Slider::new(&mut new.bottom, 0..=h / 2).text("sotto"));
                ui.end_row();
            });
            if new != old {
                if let Some(side) = aspect::changed_side(old, new) {
                    // moving one side moves the others so the window stays 16:9
                    self.edit.crop = Some(aspect::adapt(w, h, old, new, side));
                }
            }
        } else {
            ui.label(egui::RichText::new("Seleziona un file per ritagliarlo.").color(theme::MUTED));
        }
        let crop = self.edit.crop.unwrap_or_default();
        egui::Frame::new()
            .fill(theme::BG_DARK)
            .corner_radius(10.0)
            .inner_margin(6)
            .show(ui, |ui| {
                let width = ui.available_width();
                if self.running {
                    ui.vertical_centered(|ui| match &self.live {
                        Some(l) => {
                            let size = l.texture.size_vec2();
                            let shown =
                                egui::vec2(width - 4.0, (width - 4.0) * size.y / size.x.max(1.0));
                            ui.add(egui::Image::new((l.texture.id(), shown)));
                            ui.label(
                                egui::RichText::new(format!("In codifica: {}", self.live_name))
                                    .small()
                                    .color(theme::TEAL),
                            );
                        }
                        None => {
                            ui.add_space(30.0);
                            ui.label(
                                egui::RichText::new("Avvio della codifica…").color(theme::MUTED),
                            );
                            ui.add_space(30.0);
                        }
                    });
                    return;
                }
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
        o.hardsub = Some(0);
        assert!(summary(&o).ends_with("colore · sub"));
    }

    fn entry(name: &str, crop: Crop) -> Entry {
        Entry {
            path: PathBuf::from(name),
            size_mb: 1.0,
            probe: None,
            options: Options {
                crop: Some(crop),
                ..Default::default()
            },
            status: Status::Waiting,
            at: 600.0,
        }
    }

    #[test]
    fn a_quality_change_reaches_every_file_but_each_keeps_its_own_crop() {
        let mut t = VideoTab::new(&egui::Context::default(), &Settings::default());
        let (c1, c2) = (aspect::fit_frame(1920, 1080), aspect::fit_frame(1440, 1080));
        t.entries = vec![entry("a.mkv", c1), entry("b.mkv", c2)];
        t.selection.resize(2);
        t.selection.only(0);
        // nothing is selected-only: the panel edits all files
        t.edit = t.entries[0].options.clone();
        t.edit.crf = 35;
        t.apply_edit();
        assert!(t.entries.iter().all(|e| e.options.crf == 35));
        assert_eq!(t.entries[1].options.crop, Some(c2));
        // with "apply to all" off only the selection changes
        t.apply_all = false;
        t.edit.crf = 28;
        t.apply_edit();
        assert_eq!(
            (t.entries[0].options.crf, t.entries[1].options.crf),
            (28, 35)
        );
        assert_eq!(t.entries[1].at, 600.0);
    }

    #[test]
    fn status_lines_read_well() {
        assert_eq!(status_text(&Status::Running(0.5)), "In corso 50%");
        assert!(status_text(&Status::Failed("x".into())).starts_with('✖'));
    }
}
