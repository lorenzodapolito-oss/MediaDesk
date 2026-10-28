//! The Audio tab: file list with ratings, tag/cover/lyrics preview, player, autotag, rename,
//! conversion to MP3 and removal to the recycle bin.

use super::widgets::{notice_color, stars_text, texture_from_bytes, Notice, Selection};
use super::Tools;
use crate::audio::online::{self, Candidate, Client, Identified};
use crate::audio::player::{seek_target, Playback};
use crate::audio::tags::{self, AudioInfo, TagEdit};
use crate::audio::{self, convert};
use crate::ffmpeg::{self, Cancel};
use crate::settings::Settings;
use crate::theme;
use crate::util::{clock, open_folder, sanitize_filename, unique_path};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

struct Row {
    path: PathBuf,
    name: String,
    saved: u8,
    pending: Option<u8>,
}

impl Row {
    fn stars(&self) -> u8 {
        self.pending.unwrap_or(self.saved)
    }
}

enum Ev {
    Ratings(u64, Vec<u8>),
    Identified(PathBuf, Identified),
    Lyrics(PathBuf, Option<String>),
    Progress {
        done: usize,
        total: usize,
        label: String,
        fraction: f32,
    },
    Finished(Notice),
}

struct Job {
    label: String,
    done: usize,
    total: usize,
    current: String,
    fraction: f32,
    cancel: Cancel,
}

/// The suggested-tags window.
struct Review {
    path: PathBuf,
    name: String,
    found: Identified,
    /// 0 = merged suggestion, 1.. = candidates in order
    choice: usize,
    edit: Fields,
    cover: Option<Vec<u8>>,
    texture: Option<egui::TextureHandle>,
    use_cover: bool,
}

#[derive(Default, Clone)]
struct Fields {
    title: String,
    artist: String,
    album: String,
    year: String,
    genre: String,
}

impl Fields {
    fn from_candidate(c: &Candidate) -> Self {
        Self {
            title: c.title.clone(),
            artist: c.artist.clone(),
            album: c.album.clone(),
            year: c.year.clone(),
            genre: c.genre.clone(),
        }
    }
    fn from_info(i: &AudioInfo) -> Self {
        Self {
            title: i.title.clone(),
            artist: i.artist.clone(),
            album: i.album.clone(),
            year: i.year.clone(),
            genre: i.genre.clone(),
        }
    }
    fn edit(&self) -> TagEdit {
        TagEdit {
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            year: self.year.clone(),
            genre: self.genre.clone(),
            ..Default::default()
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("fields")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                for (label, value) in [
                    ("Titolo", &mut self.title),
                    ("Artista", &mut self.artist),
                    ("Album", &mut self.album),
                    ("Anno", &mut self.year),
                    ("Genere", &mut self.genre),
                ] {
                    ui.label(label);
                    ui.add(egui::TextEdit::singleline(value).desired_width(300.0));
                    ui.end_row();
                }
            });
    }
}

/// The "advanced properties" window.
struct Props {
    path: PathBuf,
    fields: Fields,
    cover: Option<Vec<u8>>,
    texture: Option<egui::TextureHandle>,
    cover_changed: bool,
}

enum Confirm {
    Delete(Vec<usize>),
    BatchTag(Vec<usize>),
}

pub struct AudioTab {
    ctx: egui::Context,
    folder: String,
    rows: Vec<Row>,
    sel: Selection,
    generation: u64,
    tx: Sender<Ev>,
    rx: Receiver<Ev>,
    // preview of the first selected file
    shown: Option<PathBuf>,
    info: Option<AudioInfo>,
    cover_tex: Option<egui::TextureHandle>,
    lyrics: String,
    // playback
    player: Option<Playback>,
    playing: Option<usize>,
    length: f64,
    volume: f32,
    // work in progress
    job: Option<Job>,
    searching: Option<String>,
    review: Option<Review>,
    props: Option<Props>,
    confirm: Option<Confirm>,
    status: Option<Notice>,
    agent: String,
    min_score: u32,
}

impl AudioTab {
    pub fn new(ctx: &egui::Context, settings: &Settings) -> Self {
        let (tx, rx) = channel();
        let mut tab = Self {
            ctx: ctx.clone(),
            folder: settings.audio_folder.clone(),
            rows: Vec::new(),
            sel: Selection::default(),
            generation: 0,
            tx,
            rx,
            shown: None,
            info: None,
            cover_tex: None,
            lyrics: String::new(),
            player: None,
            playing: None,
            length: 0.0,
            volume: 1.0,
            job: None,
            searching: None,
            review: None,
            props: None,
            confirm: None,
            status: None,
            agent: settings.musicbrainz_agent.clone(),
            min_score: settings.mb_min_score,
        };
        if !tab.folder.is_empty() {
            tab.refresh();
        }
        tab
    }

    pub fn store_settings(&self, s: &mut Settings) {
        s.audio_folder = self.folder.clone();
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
            || self.searching.is_some()
            || self
                .player
                .as_ref()
                .is_some_and(|p| !p.finished() && !p.is_paused())
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Notice {
            text: text.into(),
            error,
        });
    }

    // ---- folder / list

    fn refresh(&mut self) {
        self.stop();
        self.generation += 1;
        self.rows.clear();
        self.sel.resize(0);
        self.shown = None;
        self.info = None;
        self.cover_tex = None;
        self.lyrics.clear();
        let folder = PathBuf::from(self.folder.trim());
        if self.folder.trim().is_empty() {
            return;
        }
        match audio::list_folder(&folder) {
            Ok(files) => {
                self.rows = files
                    .into_iter()
                    .map(|p| Row {
                        name: p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        path: p,
                        saved: 0,
                        pending: None,
                    })
                    .collect();
                self.sel.resize(self.rows.len());
                // ratings are read in the background
                let (generation, tx, ctx) = (self.generation, self.tx.clone(), self.ctx.clone());
                let paths: Vec<PathBuf> = self.rows.iter().map(|r| r.path.clone()).collect();
                std::thread::spawn(move || {
                    let ratings = paths
                        .iter()
                        .map(|p| tags::read(p).map(|i| i.rating).unwrap_or(0))
                        .collect();
                    let _ = tx.send(Ev::Ratings(generation, ratings));
                    ctx.request_repaint();
                });
                self.say(format!("{} file audio", self.rows.len()), false);
            }
            Err(e) => self.say(format!("Cartella non leggibile: {e}"), true),
        }
    }

    fn browse(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Seleziona cartella con file audio")
            .pick_folder()
        {
            self.folder = p.to_string_lossy().into_owned();
            self.refresh();
        }
    }

    fn selected_paths(&self) -> Vec<(usize, PathBuf)> {
        self.sel
            .indices()
            .into_iter()
            .filter_map(|i| self.rows.get(i).map(|r| (i, r.path.clone())))
            .collect()
    }

    // ---- background events

    fn poll(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Ev::Ratings(g, ratings) if g == self.generation => {
                    for (row, r) in self.rows.iter_mut().zip(ratings) {
                        row.saved = r;
                    }
                }
                Ev::Ratings(..) => {}
                Ev::Identified(path, found) => {
                    self.searching = None;
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let merged = online::merge(&found);
                    let cover = merged.cover.clone();
                    let texture = cover
                        .as_deref()
                        .and_then(|b| texture_from_bytes(&self.ctx, "review-cover", b, 400));
                    self.review = Some(Review {
                        path,
                        name,
                        edit: Fields::from_candidate(&merged),
                        found,
                        choice: 0,
                        use_cover: cover.is_some(),
                        cover,
                        texture,
                    });
                }
                Ev::Lyrics(path, text) => {
                    if self.shown.as_deref() == Some(path.as_path()) {
                        match text {
                            Some(t) => {
                                self.lyrics = t;
                                self.say(
                                    "Testo trovato (premi «Salva testo» per scriverlo nel file)",
                                    false,
                                );
                            }
                            None => self.say("Testo non trovato online", true),
                        }
                    }
                }
                Ev::Progress {
                    done,
                    total,
                    label,
                    fraction,
                } => {
                    if let Some(j) = &mut self.job {
                        j.done = done;
                        j.total = total;
                        j.current = label;
                        j.fraction = fraction;
                    }
                }
                Ev::Finished(n) => {
                    self.job = None;
                    self.status = Some(n);
                    self.shown = None; // reload the preview: tags may have changed
                    self.refresh_ratings_only();
                }
            }
        }
    }

    fn refresh_ratings_only(&mut self) {
        let (generation, tx, ctx) = (self.generation, self.tx.clone(), self.ctx.clone());
        let paths: Vec<PathBuf> = self.rows.iter().map(|r| r.path.clone()).collect();
        std::thread::spawn(move || {
            let ratings = paths
                .iter()
                .map(|p| tags::read(p).map(|i| i.rating).unwrap_or(0))
                .collect();
            let _ = tx.send(Ev::Ratings(generation, ratings));
            ctx.request_repaint();
        });
    }

    // ---- preview

    fn update_preview(&mut self) {
        let want = self
            .sel
            .first()
            .and_then(|i| self.rows.get(i))
            .map(|r| r.path.clone());
        if want == self.shown {
            return;
        }
        self.shown = want.clone();
        self.info = None;
        self.cover_tex = None;
        self.lyrics.clear();
        let Some(path) = want else { return };
        match tags::read(&path) {
            Ok(info) => {
                self.cover_tex = info
                    .cover
                    .as_deref()
                    .and_then(|b| texture_from_bytes(&self.ctx, "cover", b, 400));
                self.lyrics = info.lyrics.clone();
                self.info = Some(info);
            }
            Err(e) => self.say(format!("Tag non leggibili: {e}"), true),
        }
    }

    // ---- player

    fn play_row(&mut self, i: usize, from: Duration) {
        self.stop();
        let Some(row) = self.rows.get(i) else { return };
        match Playback::start(&row.path, from) {
            Ok(p) => {
                p.set_volume(self.volume);
                self.length = tags::read(&row.path)
                    .map(|t| t.duration_secs)
                    .unwrap_or(0.0);
                self.player = Some(p);
                self.playing = Some(i);
            }
            Err(e) => self.say(format!("Riproduzione non riuscita: {e}"), true),
        }
    }

    fn stop(&mut self) {
        self.player = None;
        self.playing = None;
    }

    fn toggle_play(&mut self) {
        if let (Some(p), Some(_)) = (&self.player, self.playing) {
            if !p.finished() {
                if p.is_paused() {
                    p.resume();
                } else {
                    p.pause();
                }
                return;
            }
        }
        if let Some(i) = self.sel.first() {
            self.play_row(i, Duration::ZERO);
        }
    }

    /// Moves within the track and keeps playing: a paused or just-ended track resumes at once.
    fn seek_by(&mut self, delta: f64) {
        let Some(p) = &mut self.player else { return };
        let len = (self.length > 0.0).then(|| Duration::from_secs_f64(self.length));
        let to = seek_target(p.position(), delta, len);
        if p.finished() {
            // the track already ended: start it again from the new position
            if let Some(i) = self.playing {
                self.play_row(i, to);
            }
            return;
        }
        match p.seek(to) {
            Ok(()) => p.resume(),
            Err(e) => {
                let msg = format!("Spostamento non riuscito: {e}");
                self.say(msg, true);
            }
        }
    }

    fn step_track(&mut self, delta: isize) {
        let from = self.playing.or_else(|| self.sel.first());
        if let Some(i) = self.sel.step(from, delta) {
            self.sel.only(i);
            self.play_row(i, Duration::ZERO);
        }
    }

    // ---- actions

    fn save_ratings(&mut self) {
        let (mut saved, mut failed) = (0, 0);
        for row in &mut self.rows {
            if let Some(r) = row.pending {
                match tags::write_rating(&row.path, r) {
                    Ok(()) => {
                        row.saved = r;
                        row.pending = None;
                        saved += 1;
                    }
                    Err(_) => failed += 1,
                }
            }
        }
        if saved + failed == 0 {
            self.say("Nessuna modifica da salvare", false);
        } else {
            self.say(
                format!("Rating salvati: {saved}, non riusciti: {failed}"),
                failed > 0,
            );
        }
    }

    fn rename_selected(&mut self) {
        let items = self.selected_paths();
        if items.is_empty() {
            self.say("Seleziona prima dei file", true);
            return;
        }
        self.stop();
        let folder = PathBuf::from(self.folder.trim());
        let (mut renamed, mut errors) = (0, 0);
        for (i, path) in items {
            let info = tags::read(&path).unwrap_or_default();
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let base = if !info.artist.is_empty() && !info.title.is_empty() {
                format!("{} - {}", info.artist, info.title)
            } else {
                format!("{stem}_edited")
            };
            let target = unique_path(&folder, &sanitize_filename(&base), &ext);
            if target == path {
                continue;
            }
            match std::fs::rename(&path, &target) {
                Ok(()) => {
                    if let Some(row) = self.rows.get_mut(i) {
                        row.name = target
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        row.path = target;
                    }
                    renamed += 1;
                }
                Err(_) => errors += 1,
            }
        }
        self.shown = None;
        self.say(
            format!("File rinominati: {renamed}, errori: {errors}"),
            errors > 0,
        );
    }

    fn delete_rows(&mut self, mut idx: Vec<usize>) {
        self.stop();
        idx.sort_unstable();
        let mut deleted = 0;
        for &i in idx.iter().rev() {
            let Some(row) = self.rows.get(i) else {
                continue;
            };
            if trash::delete(&row.path).is_ok() {
                self.rows.remove(i);
                deleted += 1;
            }
        }
        self.sel.resize(self.rows.len());
        self.shown = None;
        self.say(
            format!("File spostati nel cestino: {deleted}"),
            deleted == 0,
        );
    }

    fn start_convert(&mut self, tools: &Tools) {
        let Some(ff) = tools.ffmpeg.clone() else {
            self.say("ffmpeg non trovato: impostalo in Impostazioni", true);
            return;
        };
        let items = self.selected_paths();
        if items.is_empty() {
            self.say("Seleziona prima dei file", true);
            return;
        }
        let out_dir = PathBuf::from(self.folder.trim()).join("convertiti");
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            self.say(format!("Cartella di destinazione non creata: {e}"), true);
            return;
        }
        let cancel = ffmpeg::new_cancel();
        self.job = Some(Job {
            label: "Conversione MP3 160k + normalizzazione".into(),
            done: 0,
            total: items.len(),
            current: String::new(),
            fraction: 0.0,
            cancel: cancel.clone(),
        });
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let total = items.len();
            let (mut ok, mut failed) = (0, 0);
            for (n, (_, path)) in items.iter().enumerate() {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let length = tags::read(path).map(|i| i.duration_secs).unwrap_or(0.0);
                let _ = tx.send(Ev::Progress {
                    done: n,
                    total,
                    label: name.clone(),
                    fraction: 0.0,
                });
                ctx.request_repaint();
                let out = convert::output_path(path, &out_dir);
                let args = convert::args(path, &out);
                let res = ffmpeg::run(&ff, &args, &cancel, |secs| {
                    let f = if length > 0.0 {
                        (secs / length).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    let _ = tx.send(Ev::Progress {
                        done: n,
                        total,
                        label: name.clone(),
                        fraction: f,
                    });
                    ctx.request_repaint();
                });
                match res {
                    Ok(o) if o.success => ok += 1,
                    Ok(o) => {
                        let _ = std::fs::remove_file(&out);
                        if o.cancelled {
                            break;
                        }
                        failed += 1;
                    }
                    Err(_) => failed += 1,
                }
            }
            let cancelled = cancel.load(std::sync::atomic::Ordering::Relaxed);
            let text = if cancelled {
                format!("Conversione interrotta ({ok} completati)")
            } else {
                format!(
                    "Conversione finita: {ok} ok, {failed} errori: {}",
                    out_dir.display()
                )
            };
            let _ = tx.send(Ev::Finished(Notice {
                text,
                error: failed > 0,
            }));
            ctx.request_repaint();
        });
    }

    fn start_autotag(&mut self) {
        let items = self.selected_paths();
        match items.len() {
            0 => self.say("Seleziona prima dei file", true),
            1 => {
                let path = items[0].1.clone();
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.searching = Some(name.clone());
                let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
                let (agent, score) = (self.agent.clone(), self.min_score);
                std::thread::spawn(move || {
                    let mut client = Client::new(&agent, score);
                    let found = client.identify(&name, true, |_| {});
                    let _ = tx.send(Ev::Identified(path, found));
                    ctx.request_repaint();
                });
            }
            _ => {
                self.confirm = Some(Confirm::BatchTag(
                    items.into_iter().map(|(i, _)| i).collect(),
                ))
            }
        }
    }

    fn run_batch_tag(&mut self, idx: Vec<usize>) {
        let items: Vec<PathBuf> = idx
            .iter()
            .filter_map(|&i| self.rows.get(i).map(|r| r.path.clone()))
            .collect();
        let cancel = ffmpeg::new_cancel();
        self.job = Some(Job {
            label: "Autotag".into(),
            done: 0,
            total: items.len(),
            current: String::new(),
            fraction: 0.0,
            cancel: cancel.clone(),
        });
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        let (agent, score) = (self.agent.clone(), self.min_score);
        std::thread::spawn(move || {
            let mut client = Client::new(&agent, score);
            let total = items.len();
            let (mut ok, mut failed) = (0, 0);
            for (n, path) in items.iter().enumerate() {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let _ = tx.send(Ev::Progress {
                    done: n,
                    total,
                    label: name.clone(),
                    fraction: 0.0,
                });
                ctx.request_repaint();
                let found = client.identify(&name, true, |_| {});
                let m = online::merge(&found);
                let edit = TagEdit {
                    title: m.title,
                    artist: m.artist,
                    album: m.album,
                    year: m.year,
                    genre: m.genre,
                    cover: m.cover,
                    ..Default::default()
                };
                match tags::write(path, &edit) {
                    Ok(()) => ok += 1,
                    Err(_) => failed += 1,
                }
            }
            let _ = tx.send(Ev::Finished(Notice {
                text: format!("Autotag finito: {ok} ok, {failed} errori"),
                error: failed > 0,
            }));
            ctx.request_repaint();
        });
    }

    fn fetch_lyrics(&mut self) {
        let (Some(path), Some(info)) = (self.shown.clone(), self.info.clone()) else {
            return;
        };
        let guess = audio::guess::guess_from_filename(
            &path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        let artist = if info.artist.is_empty() {
            guess.artist
        } else {
            info.artist
        };
        let title = if info.title.is_empty() {
            guess.title
        } else {
            info.title
        };
        let (tx, ctx, agent, score) = (
            self.tx.clone(),
            self.ctx.clone(),
            self.agent.clone(),
            self.min_score,
        );
        self.say("Cerco il testo…", false);
        std::thread::spawn(move || {
            let text = Client::new(&agent, score).lyrics(&artist, &title);
            let _ = tx.send(Ev::Lyrics(path, text));
            ctx.request_repaint();
        });
    }

    fn save_lyrics(&mut self) {
        let Some(path) = self.shown.clone() else {
            return;
        };
        let edit = TagEdit {
            lyrics: Some(self.lyrics.clone()),
            ..Default::default()
        };
        match tags::write(&path, &edit) {
            Ok(()) => self.say("Testo salvato nel file", false),
            Err(e) => self.say(format!("Testo non salvato: {e}"), true),
        }
    }

    fn open_props(&mut self) {
        let Some(path) = self
            .sel
            .first()
            .and_then(|i| self.rows.get(i))
            .map(|r| r.path.clone())
        else {
            self.say("Seleziona prima un file", true);
            return;
        };
        let info = tags::read(&path).unwrap_or_default();
        let texture = info
            .cover
            .as_deref()
            .and_then(|b| texture_from_bytes(&self.ctx, "props-cover", b, 300));
        self.props = Some(Props {
            path,
            fields: Fields::from_info(&info),
            cover: info.cover,
            texture,
            cover_changed: false,
        });
    }

    // ---- drawing

    pub fn ui(&mut self, ui: &mut egui::Ui, tools: &Tools, _settings: &Settings) {
        self.poll();
        self.update_preview();
        let ctx = ui.ctx().clone();
        if self.player.as_ref().is_some_and(|p| p.finished()) {
            self.stop();
        }
        self.keys(&ctx, tools);

        let mut act: Vec<Act> = Vec::new();
        egui::Panel::top("audio_top")
            .frame(theme::page(10).inner_margin(egui::Margin {
                left: 10,
                right: 10,
                top: 10,
                bottom: 0,
            }))
            .show(ui, |ui| {
                theme::card()
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            if theme::button(ui, "Sfoglia", theme::PRIMARY).clicked() {
                                act.push(Act::Browse);
                            }
                            ui.label("📂 Percorso:");
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut self.folder)
                                    .desired_width(ui.available_width() - 150.0),
                            );
                            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                act.push(Act::Refresh);
                            }
                            if theme::button(ui, "📁 Apri cartella", theme::PRIMARY).clicked() {
                                act.push(Act::OpenFolder);
                            }
                        });
                    });
            });
        egui::Panel::bottom("audio_status")
            .frame(theme::page(10).inner_margin(egui::Margin::symmetric(16, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(j) = &self.job {
                        ui.label(format!(
                            "{} — {}/{} {}",
                            j.label,
                            (j.done + 1).min(j.total),
                            j.total,
                            j.current
                        ));
                        let overall = (j.done as f32 + j.fraction) / j.total.max(1) as f32;
                        ui.add(
                            egui::ProgressBar::new(overall.clamp(0.0, 1.0))
                                .desired_width(220.0)
                                .fill(theme::TEAL),
                        );
                        if theme::button(ui, "Ferma", theme::ACCENT).clicked() {
                            act.push(Act::CancelJob);
                        }
                    } else if let Some(name) = &self.searching {
                        ui.spinner();
                        ui.label(format!("Identifico {name}…"));
                    } else if let Some(n) = &self.status {
                        ui.colored_label(notice_color(n), &n.text);
                    }
                });
            });
        egui::Panel::left("audio_left")
            .exact_size(270.0)
            .frame(theme::page(10))
            .show(ui, |ui| {
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.0;
                    for (text, color, a) in [
                        ("✅ Seleziona tutti", theme::PRIMARY, Act::SelectAll),
                        ("💿 Converti MP3 160k + norm", theme::TEAL, Act::Convert),
                        ("🤖 Autotag automatico", theme::PURPLE, Act::AutoTag),
                        ("📝 Proprietà avanzate", theme::BLUE, Act::Props),
                        ("✏️ Rinomina file", theme::VIOLET, Act::Rename),
                        ("🔥 Salva modifiche", theme::ORANGE, Act::SaveRatings),
                    ] {
                        if theme::wide_button(ui, text, color).clicked() {
                            act.push(a);
                        }
                    }
                });
                ui.add_space(10.0);
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.player_controls(ui, &mut act);
                });
            });
        egui::Panel::right("audio_right")
            .default_size(370.0)
            .frame(theme::page(10))
            .show(ui, |ui| {
                self.preview(ui, &mut act);
            });
        egui::CentralPanel::default()
            .frame(theme::page(10))
            .show(ui, |ui| {
                theme::list_card().show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    self.table(ui, &mut act);
                });
            });
        self.windows(&ctx);
        for a in act {
            self.apply(a, tools);
        }
    }

    fn keys(&mut self, ctx: &egui::Context, tools: &Tools) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::Key;
        let (right, left, enter, space, up, down, del, esc, all, save) = ctx.input(|i| {
            let c = i.modifiers.command;
            (
                i.key_pressed(Key::ArrowRight),
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Space),
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::Delete),
                i.key_pressed(Key::Escape),
                c && i.key_pressed(Key::A),
                c && i.key_pressed(Key::S),
            )
        });
        if right {
            self.apply(Act::Seek(30.0), tools);
        }
        if left {
            self.apply(Act::Seek(-30.0), tools);
        }
        if enter || space {
            self.apply(Act::Toggle, tools);
        }
        if up {
            self.apply(Act::Prev, tools);
        }
        if down {
            self.apply(Act::Next, tools);
        }
        if del {
            self.apply(Act::Delete, tools);
        }
        if esc {
            self.apply(Act::Stop, tools);
        }
        if all {
            self.apply(Act::SelectAll, tools);
        }
        if save {
            self.apply(Act::SaveRatings, tools);
        }
    }

    fn apply(&mut self, a: Act, tools: &Tools) {
        match a {
            Act::Browse => self.browse(),
            Act::Refresh => self.refresh(),
            Act::OpenFolder => {
                if let Err(e) = open_folder(Path::new(self.folder.trim())) {
                    self.say(format!("Cartella non aperta: {e}"), true);
                }
            }
            Act::SelectAll => self.sel.select_all(),
            Act::Convert => self.start_convert(tools),
            Act::AutoTag => self.start_autotag(),
            Act::Props => self.open_props(),
            Act::Rename => self.rename_selected(),
            Act::SaveRatings => self.save_ratings(),
            Act::Toggle => self.toggle_play(),
            Act::Stop => self.stop(),
            Act::Prev => self.step_track(-1),
            Act::Next => self.step_track(1),
            Act::Seek(d) => self.seek_by(d),
            Act::Delete => {
                let idx = self.sel.indices();
                if !idx.is_empty() {
                    self.confirm = Some(Confirm::Delete(idx));
                }
            }
            Act::CancelJob => {
                if let Some(j) = &self.job {
                    j.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Act::PlayRow(i) => self.play_row(i, Duration::ZERO),
            Act::FetchLyrics => self.fetch_lyrics(),
            Act::SaveLyrics => self.save_lyrics(),
            Act::SeekTo(t) => {
                let to = Duration::from_secs_f64(t.max(0.0));
                if self.player.as_ref().is_some_and(|p| p.finished()) {
                    if let Some(i) = self.playing {
                        self.play_row(i, to);
                    }
                } else if let Some(p) = &mut self.player {
                    if p.seek(to).is_ok() {
                        p.resume();
                    }
                }
            }
        }
    }

    fn player_controls(&mut self, ui: &mut egui::Ui, act: &mut Vec<Act>) {
        use egui::{vec2, RichText};
        ui.horizontal(|ui| {
            theme::section(ui, "Player");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::button_sized(ui, vec2(38.0, 28.0), RichText::new("🗑"), theme::ACCENT)
                    .on_hover_text("Sposta nel cestino (Canc)")
                    .clicked()
                {
                    act.push(Act::Delete);
                }
            });
        });
        ui.add_space(2.0);
        let w = ui.available_width();
        let ghost = theme::FIELD_BG;
        if theme::button_sized(
            ui,
            vec2(w, 28.0),
            RichText::new("▲ Precedente").size(13.0),
            ghost,
        )
        .clicked()
        {
            act.push(Act::Prev);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if theme::button_sized(
                ui,
                vec2(46.0, 34.0),
                RichText::new("-30s").size(13.0),
                ghost,
            )
            .clicked()
            {
                act.push(Act::Seek(-30.0));
            }
            let paused_or_idle = self.player.as_ref().is_none_or(|p| p.is_paused());
            let label = if paused_or_idle { "▶" } else { "⏸" };
            let color = if paused_or_idle {
                theme::TEAL
            } else {
                theme::ACCENT
            };
            if theme::button_sized(ui, vec2(60.0, 42.0), RichText::new(label).size(18.0), color)
                .clicked()
            {
                act.push(Act::Toggle);
            }
            if theme::button_sized(ui, vec2(40.0, 34.0), RichText::new("⏹"), theme::ACCENT)
                .clicked()
            {
                act.push(Act::Stop);
            }
            if theme::button_sized(
                ui,
                vec2(46.0, 34.0),
                RichText::new("+30s").size(13.0),
                ghost,
            )
            .clicked()
            {
                act.push(Act::Seek(30.0));
            }
        });
        if theme::button_sized(
            ui,
            vec2(w, 28.0),
            RichText::new("▼ Successivo").size(13.0),
            ghost,
        )
        .clicked()
        {
            act.push(Act::Next);
        }
        if let Some(p) = &self.player {
            let pos = p.position().as_secs_f64();
            let mut t = pos;
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(clock(pos)).small().color(theme::MUTED));
                ui.spacing_mut().slider_width = (ui.available_width() - 46.0).max(60.0);
                let r =
                    ui.add(egui::Slider::new(&mut t, 0.0..=self.length.max(1.0)).show_value(false));
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    act.push(Act::SeekTo(t));
                }
                ui.label(
                    RichText::new(clock(self.length))
                        .small()
                        .color(theme::MUTED),
                );
            });
            if let Some(name) = self
                .playing
                .and_then(|i| self.rows.get(i))
                .map(|r| r.name.clone())
            {
                ui.label(RichText::new(name).small().color(theme::MUTED));
            }
        }
        ui.horizontal(|ui| {
            ui.label("🔊");
            ui.spacing_mut().slider_width = (ui.available_width() - 8.0).max(60.0);
            if ui
                .add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false))
                .changed()
            {
                if let Some(p) = &self.player {
                    p.set_volume(self.volume);
                }
            }
        });
    }

    fn preview(&mut self, ui: &mut egui::Ui, act: &mut Vec<Act>) {
        let frame = theme::card();
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            match (&self.info, &self.shown) {
                (Some(i), Some(_)) => {
                    let or = |s: &str| {
                        if s.is_empty() {
                            "N/D".to_string()
                        } else {
                            s.to_string()
                        }
                    };
                    ui.label(format!("🎵 Titolo: {}", or(&i.title)));
                    ui.label(format!("🎤 Artista: {}", or(&i.artist)));
                    ui.label(format!("💿 Album: {}", or(&i.album)));
                    ui.label(format!("📅 Anno: {}", or(&i.year)));
                    ui.label(format!("🎼 Genere: {}", or(&i.genre)));
                    ui.label(format!(
                        "⏱ Durata: {}   {}",
                        clock(i.duration_secs),
                        stars_text(i.rating)
                    ));
                }
                (None, Some(_)) => {
                    ui.label("⚠️ Tag non leggibili");
                }
                _ => {
                    ui.label(egui::RichText::new("Seleziona un file").color(theme::MUTED));
                }
            }
        });
        ui.add_space(6.0);
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| match &self.cover_tex {
                Some(t) => {
                    let size = t.size_vec2();
                    let scale = (260.0 / size.x.max(size.y)).min(1.0);
                    ui.add(egui::Image::new((t.id(), size * scale)));
                }
                None => {
                    ui.add_space(30.0);
                    ui.label(egui::RichText::new("Anteprima copertina").color(theme::MUTED));
                    ui.add_space(30.0);
                }
            });
        });
        ui.add_space(6.0);
        theme::card()
            .fill(egui::Color32::from_rgb(0x1e, 0x2b, 0x37))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label("📜 Testo");
                    if ui.small_button("Cerca online").clicked() {
                        act.push(Act::FetchLyrics);
                    }
                    if ui.small_button("Salva testo").clicked() {
                        act.push(Act::SaveLyrics);
                    }
                });
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.lyrics)
                            .desired_width(f32::INFINITY)
                            .desired_rows(14),
                    );
                });
            });
    }

    fn table(&mut self, ui: &mut egui::Ui, act: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("NOME FILE")
                    .small()
                    .strong()
                    .color(theme::MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("RATING")
                        .small()
                        .strong()
                        .color(theme::MUTED),
                );
            });
        });
        ui.add_space(2.0);
        let row_h = 32.0;
        let n = self.rows.len();
        let mods = ui.input(|i| i.modifiers);
        let mut clicked: Option<usize> = None;
        let mut dbl: Option<usize> = None;
        let mut star_click: Option<(usize, u8)> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, n, |ui, range| {
                for i in range {
                    let Some(row) = self.rows.get(i) else {
                        continue;
                    };
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), row_h),
                        egui::Sense::click(),
                    );
                    let selected = self.sel.contains(i);
                    let playing = self.playing == Some(i);
                    let bg = rect.shrink2(egui::vec2(2.0, 1.0));
                    if selected {
                        ui.painter().rect_filled(bg, 9.0, theme::TEAL);
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(bg, 9.0, theme::lighten(theme::LIST_BG, 0.1));
                    } else if i % 2 == 1 {
                        ui.painter()
                            .rect_filled(bg, 9.0, theme::darken(theme::LIST_BG, 0.08));
                    }
                    let font = egui::FontId::proportional(14.0);
                    let prefix = if playing { "▶ " } else { "" };
                    ui.painter().text(
                        rect.left_center() + egui::vec2(12.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{prefix}{}", row.name),
                        font.clone(),
                        theme::TEXT,
                    );
                    let star_w = 18.0;
                    let stars_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.right() - 5.0 * star_w - 12.0, rect.top()),
                        egui::vec2(5.0 * star_w, row_h),
                    );
                    let txt = stars_text(row.stars());
                    let tint = if row.pending.is_some() {
                        theme::ORANGE
                    } else {
                        theme::TEXT
                    };
                    ui.painter().text(
                        stars_rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        txt,
                        font,
                        tint,
                    );
                    if resp.double_clicked() {
                        dbl = Some(i);
                    } else if resp.clicked() {
                        match resp.interact_pointer_pos() {
                            Some(p) if stars_rect.contains(p) => {
                                let n = (((p.x - stars_rect.left()) / star_w).floor() as i32 + 1)
                                    .clamp(1, 5) as u8;
                                star_click = Some((i, n));
                            }
                            _ => clicked = Some(i),
                        }
                    }
                }
            });
        if let Some(i) = clicked {
            self.sel.click(i, mods.command, mods.shift);
        }
        if let Some(i) = dbl {
            self.sel.only(i);
            act.push(Act::PlayRow(i));
        }
        if let Some((i, n)) = star_click {
            if let Some(row) = self.rows.get_mut(i) {
                let new = if row.stars() == n { 0 } else { n };
                row.pending = (new != row.saved).then_some(new);
            }
            self.sel.only(i);
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        // confirmations
        let mut answer: Option<bool> = None;
        if let Some(c) = &self.confirm {
            let (title, text) = match c {
                Confirm::Delete(v) => ("Conferma eliminazione", format!("Spostare {} file nel cestino?\n\nPotrai recuperarli dal cestino.", v.len())),
                Confirm::BatchTag(v) => ("Autotag batch", format!("Identificare e applicare automaticamente i tag a {} file?\n\nI tag esistenti verranno sovrascritti.", v.len())),
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(text);
                    ui.horizontal(|ui| {
                        if theme::button(ui, "Sì", theme::PRIMARY).clicked() {
                            answer = Some(true);
                        }
                        if theme::button(ui, "No", theme::ACCENT).clicked() {
                            answer = Some(false);
                        }
                    });
                });
        }
        if let Some(yes) = answer {
            let c = self.confirm.take();
            if yes {
                match c {
                    Some(Confirm::Delete(v)) => self.delete_rows(v),
                    Some(Confirm::BatchTag(v)) => self.run_batch_tag(v),
                    None => {}
                }
            }
        }
        self.review_window(ctx);
        self.props_window(ctx);
    }

    fn review_window(&mut self, ctx: &egui::Context) {
        let Some(r) = &mut self.review else { return };
        let mut apply = false;
        let mut close = false;
        let mut open = true;
        let mut pick: Option<usize> = None;
        egui::Window::new(format!("Tag suggeriti: {}", r.name))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Fonte").strong());
                        if ui
                            .selectable_label(r.choice == 0, "★ Unione (consigliato)")
                            .clicked()
                        {
                            pick = Some(0);
                        }
                        for (n, c) in r.found.candidates.iter().enumerate() {
                            let label = format!("{}: {} – {}", c.source, c.artist, c.title);
                            if ui.selectable_label(r.choice == n + 1, label).clicked() {
                                pick = Some(n + 1);
                            }
                        }
                        if r.found.candidates.is_empty() {
                            ui.colored_label(
                                theme::ACCENT,
                                "Nessuna fonte ha risposto: uso il nome del file.",
                            );
                        }
                    });
                    ui.vertical(|ui| match &r.texture {
                        Some(t) => {
                            let s = t.size_vec2();
                            ui.add(egui::Image::new((
                                t.id(),
                                s * (160.0 / s.x.max(s.y)).min(1.0),
                            )));
                            ui.checkbox(&mut r.use_cover, "Scrivi la copertina");
                        }
                        None => {
                            ui.label(
                                egui::RichText::new("(nessuna copertina)").color(theme::MUTED),
                            );
                        }
                    });
                });
                ui.separator();
                r.edit.ui(ui);
                ui.horizontal(|ui| {
                    if theme::button(ui, "Applica", theme::PRIMARY).clicked() {
                        apply = true;
                    }
                    if theme::button(ui, "Annulla", theme::ACCENT).clicked() {
                        close = true;
                    }
                });
            });
        if let Some(p) = pick {
            r.choice = p;
            let c = if p == 0 {
                online::merge(&r.found)
            } else {
                r.found.candidates.get(p - 1).cloned().unwrap_or_default()
            };
            r.edit = Fields::from_candidate(&c);
            r.cover = c.cover.clone();
            r.texture = r
                .cover
                .as_deref()
                .and_then(|b| texture_from_bytes(ctx, "review-cover", b, 400));
            r.use_cover = r.cover.is_some();
        }
        if apply {
            let mut edit = r.edit.edit();
            if r.use_cover {
                edit.cover = r.cover.clone();
            }
            let path = r.path.clone();
            match tags::write(&path, &edit) {
                Ok(()) => {
                    self.review = None;
                    self.shown = None;
                    self.say("Tag applicati", false);
                }
                Err(e) => self.say(format!("Tag non scritti: {e}"), true),
            }
        } else if close || !open {
            self.review = None;
        }
    }

    fn props_window(&mut self, ctx: &egui::Context) {
        let Some(p) = &mut self.props else { return };
        let (mut apply, mut close, mut open) = (false, false, true);
        let name = p
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        egui::Window::new(format!("Proprietà: {name}"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                p.fields.ui(ui);
                ui.horizontal(|ui| {
                    if let Some(t) = &p.texture {
                        let s = t.size_vec2();
                        ui.add(egui::Image::new((
                            t.id(),
                            s * (120.0 / s.x.max(s.y)).min(1.0),
                        )));
                    }
                    if ui.button("Scegli copertina…").clicked() {
                        if let Some(f) = rfd::FileDialog::new()
                            .add_filter("Immagini", &["jpg", "jpeg", "png"])
                            .pick_file()
                        {
                            if let Ok(bytes) = std::fs::read(&f) {
                                p.texture = texture_from_bytes(ctx, "props-cover", &bytes, 300);
                                p.cover = Some(bytes);
                                p.cover_changed = true;
                            }
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if theme::button(ui, "Salva", theme::PRIMARY).clicked() {
                        apply = true;
                    }
                    if theme::button(ui, "Chiudi", theme::ACCENT).clicked() {
                        close = true;
                    }
                });
            });
        if apply {
            let mut edit = p.fields.edit();
            if p.cover_changed {
                edit.cover = p.cover.clone();
            }
            let path = p.path.clone();
            match tags::write(&path, &edit) {
                Ok(()) => {
                    self.props = None;
                    self.shown = None;
                    self.say("Proprietà salvate", false);
                }
                Err(e) => self.say(format!("Proprietà non salvate: {e}"), true),
            }
        } else if close || !open {
            self.props = None;
        }
    }
}

enum Act {
    Browse,
    Refresh,
    OpenFolder,
    SelectAll,
    Convert,
    AutoTag,
    Props,
    Rename,
    SaveRatings,
    Toggle,
    Stop,
    Prev,
    Next,
    Seek(f64),
    SeekTo(f64),
    Delete,
    CancelJob,
    PlayRow(usize),
    FetchLyrics,
    SaveLyrics,
}
