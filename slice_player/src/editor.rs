//! egui editor — waveform view with interactive slice markers, three slice modes,
//! per-slice parameter editor, and MIDI export.

use std::sync::{Arc, RwLock};
use egui::{
    Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2, FontFamily,
    CursorIcon,
};
use nih_plug_egui::egui;


use crate::slicer::{DelayRate, GridDivision, RetriggerRate, SliceLoop, TransientSettings};
use crate::midi_export::{export_midi, copy_midi_file_to_clipboard};

// ── Colours ───────────────────────────────────────────────────────────────────
const BG:        Color32 = Color32::from_rgb(18,  20,  26);
const PANEL_BG:  Color32 = Color32::from_rgb(26,  29,  40);
const ACCENT:    Color32 = Color32::from_rgb(98,  160, 255);
const ACCENT2:   Color32 = Color32::from_rgb(255, 160,  60);
const WAVEFORM:  Color32 = Color32::from_rgb(60,  200, 140);
const MARKER:    Color32 = Color32::from_rgb(255,  80,  80);
const MARKER_HO: Color32 = Color32::from_rgb(255, 160,  60);
const TEXT_DIM:  Color32 = Color32::from_rgb(130, 140, 160);


// Width in pixels within which a marker is considered "grabbed".
const MARKER_GRAB_PX: f32 = 6.0;

use std::path::PathBuf;
use crate::engine::Engine;
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_audio: bool,
}

pub struct FileBrowserState {
    pub current_dir: PathBuf,
    pub entries: Vec<FileEntry>,
    pub selected_file: Option<PathBuf>,
    pub auto_play: bool,
    pub visible: bool,
}

impl FileBrowserState {
    pub fn new(initial_dir: PathBuf) -> Self {
        let mut s = Self {
            current_dir: initial_dir,
            entries: Vec::new(),
            selected_file: None,
            auto_play: true,
            visible: true,
        };
        s.refresh();
        s
    }

    pub fn refresh(&mut self) {
        self.entries.clear();
        let Ok(read_dir) = std::fs::read_dir(&self.current_dir) else { return; };
        
        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in read_dir.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = path.is_dir();
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            let is_audio = matches!(ext.as_str(), "wav" | "aif" | "aiff" | "flac" | "mp3" | "m4a" | "aac" | "opus" | "ogg" | "rx2" | "rex" | "rcy");

            if is_dir {
                dirs.push(FileEntry { name, path, is_dir: true, is_audio: false });
            } else if is_audio {
                files.push(FileEntry { name, path, is_dir: false, is_audio: true });
            }
        }

        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        self.entries.extend(dirs);
        self.entries.extend(files);
    }

    pub fn set_dir(&mut self, dir: PathBuf) {
        self.current_dir = dir;
        self.refresh();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlicingModeTab {
    Transient,
    Grid,
    Manual,
}

// ── Editor state (lives in the GUI thread, not audio thread) ─────────────────
pub struct EditorState {
    pub loop_data: Arc<RwLock<Option<SliceLoop>>>,
    pub engine: Arc<Mutex<Engine>>,
    pub last_dir: Arc<Mutex<Option<PathBuf>>>,
    pub favorites: Arc<Mutex<[Option<PathBuf>; 5]>>,
    pub file_browser: FileBrowserState,
    pub selected_slice: Option<usize>,
    pub grid_division: GridDivision,
    pub bpm_input: f64,
    pub transient_settings: TransientSettings,
    pub transient_preview: Vec<usize>,   // detected positions, shown before applying
    pub slicing_tab: SlicingModeTab,
    pub status_msg: String,
    /// Which slice marker is currently being dragged (index into slices).
    dragging_marker: Option<usize>,
    /// Which fade handle is currently being dragged: (slice_index, is_fade_out).
    dragging_fade: Option<(usize, bool)>,
    /// Which loop marker is currently being dragged: false = loop_start, true = loop_end.
    dragging_loop_bound: Option<bool>,
    /// Horizontal waveform zoom factor (1.0 = 100% full view, up to 30.0x).
    pub zoom_factor: f32,
    /// Horizontal scroll offset (normalized 0.0..=1.0).
    pub zoom_scroll: f32,
}

impl EditorState {
    pub fn new(
        loop_data: Arc<RwLock<Option<SliceLoop>>>,
        engine: Arc<Mutex<Engine>>,
        last_dir: Arc<Mutex<Option<PathBuf>>>,
        favorites: Arc<Mutex<[Option<PathBuf>; 5]>>,
    ) -> Self {
        let initial_dir = last_dir.lock().unwrap().clone()
            .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")));

        Self {
            loop_data,
            engine,
            last_dir,
            favorites,
            file_browser: FileBrowserState::new(initial_dir),
            selected_slice: None,
            grid_division: GridDivision::Eighth,
            bpm_input: 120.0,
            transient_settings: TransientSettings::default(),
            transient_preview: Vec::new(),
            slicing_tab: SlicingModeTab::Transient,
            status_msg: String::from("SlicePlayer ready."),
            dragging_marker: None,
            dragging_fade: None,
            dragging_loop_bound: None,
            zoom_factor: 1.0,
            zoom_scroll: 0.0,
        }
    }

    fn status(&mut self, msg: impl Into<String>) {
        self.status_msg = msg.into();
    }
}

// ── Top-level UI entry point ──────────────────────────────────────────────────
pub fn draw(ui: &mut Ui, state: &mut EditorState) {
    // Apply dark background.
    ui.painter().rect_filled(ui.max_rect(), 0.0, BG);

    // Handle OS File Drag and Drop.
    handle_drag_and_drop(ui, state);

    if state.file_browser.visible {
        egui::SidePanel::left("file_browser_panel")
            .resizable(true)
            .default_width(220.0)
            .width_range(160.0..=360.0)
            .frame(egui::Frame::NONE.fill(PANEL_BG).inner_margin(6.0))
            .show_inside(ui, |ui| {
                draw_file_browser(ui, state);
            });
    }

    egui::TopBottomPanel::bottom("bottom_controls_panel")
        .frame(egui::Frame::NONE.inner_margin(4.0))
        .show_inside(ui, |ui| {
            if state.selected_slice.is_some() {
                draw_slice_editor(ui, state);
                ui.add_space(3.0);
            }
            draw_slice_mode_panel(ui, state);
            ui.add_space(3.0);
            draw_status(ui, state);
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(4.0))
        .show_inside(ui, |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                draw_toolbar(ui, state);
                ui.add_space(4.0);
                draw_waveform(ui, state);
            });
        });
}

fn draw_file_browser(ui: &mut Ui, state: &mut EditorState) {
    ui.vertical(|ui| {
        // Navigation header.
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("📁 Explorer").color(ACCENT).strong());
            ui.add_space(4.0);

            // Up folder button.
            if ui.small_button("⬆ Up").clicked() {
                if let Some(parent) = state.file_browser.current_dir.parent() {
                    let parent_dir = parent.to_path_buf();
                    state.file_browser.set_dir(parent_dir.clone());
                    *state.last_dir.lock().unwrap() = Some(parent_dir);
                }
            }

            // Refresh button.
            if ui.small_button("🔄").clicked() {
                state.file_browser.refresh();
            }

            // Auto-play toggle.
            ui.checkbox(&mut state.file_browser.auto_play, "Auto");
        });

        // ── 5 Favorites Slots ────────────────────────────────────────────────
        let mut status_update: Option<String> = None;
        let mut jump_dir: Option<PathBuf> = None;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("⭐ Favs:").color(ACCENT2).strong().size(11.0));
            let current_dir = state.file_browser.current_dir.clone();
            let mut favs = state.favorites.lock().unwrap().clone();
            let mut modified = false;

            #[allow(clippy::needless_range_loop)]
            for i in 0..5 {
                let slot_path = favs[i].clone();
                let has_path = slot_path.is_some();
                let is_current = slot_path.as_ref() == Some(&current_dir);
                let (label_text, btn_color) = if is_current {
                    (format!("[{}]", i + 1), ACCENT)
                } else if has_path {
                    (format!("{}", i + 1), ACCENT2)
                } else {
                    (format!("{}", i + 1), TEXT_DIM)
                };

                let btn_resp = ui.add(egui::Button::new(
                    egui::RichText::new(label_text).color(btn_color).strong().size(10.0)
                ).small());

                let tooltip = match &slot_path {
                    Some(p) => format!("Fav #{}: {}\n• Left-Click: Jump to folder\n• Right-Click: Set current folder", i + 1, p.display()),
                    None => format!("Fav #{}: (Empty)\n• Click / Right-Click: Save current folder here", i + 1),
                };
                let btn_resp = btn_resp.on_hover_text(tooltip);

                if btn_resp.clicked() {
                    if let Some(p) = slot_path {
                        if p.exists() {
                            jump_dir = Some(p.clone());
                            status_update = Some(format!("Jumped to Fav #{}: {}", i + 1, p.display()));
                        } else {
                            status_update = Some(format!("Fav #{}: folder missing", i + 1));
                        }
                    } else {
                        favs[i] = Some(current_dir.clone());
                        modified = true;
                        status_update = Some(format!("Set Fav #{} to {}", i + 1, current_dir.display()));
                    }
                }

                if btn_resp.secondary_clicked() {
                    favs[i] = Some(current_dir.clone());
                    modified = true;
                    status_update = Some(format!("Set Fav #{} to {}", i + 1, current_dir.display()));
                }
            }

            if ui.add(egui::Button::new(egui::RichText::new("+").color(ACCENT).size(10.0)).small()).on_hover_text("Save current folder to next empty Favorite slot").clicked() {
                let mut set = false;
                for slot in favs.iter_mut() {
                    if slot.is_none() {
                        *slot = Some(current_dir.clone());
                        set = true;
                        break;
                    }
                }
                if !set {
                    favs[0] = Some(current_dir.clone());
                }
                modified = true;
                status_update = Some(format!("Saved Favorite: {}", current_dir.display()));
            }

            if modified {
                *state.favorites.lock().unwrap() = favs;
                crate::sync_global_settings(&state.last_dir, &state.favorites);
            }
        });

        if let Some(dir) = jump_dir {
            state.file_browser.set_dir(dir.clone());
            *state.last_dir.lock().unwrap() = Some(dir);
            crate::sync_global_settings(&state.last_dir, &state.favorites);
        }
        if let Some(msg) = status_update {
            state.status(msg);
        }

        // Current directory label.
        let dir_name = state.file_browser.current_dir.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| state.file_browser.current_dir.to_string_lossy().to_string());
        ui.label(egui::RichText::new(format!("📂 /{dir_name}")).color(TEXT_DIM).size(10.0));
        ui.separator();

        // Actions on selected file (Audition / Load).
        let mut load_path: Option<PathBuf> = None;
        let mut audition_path: Option<PathBuf> = None;

        if let Some(sel) = &state.file_browser.selected_file {
            let filename = sel.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&filename).color(ACCENT2).strong().size(11.0));
                let path = sel.clone();

                let is_auditioning = state.engine.lock().unwrap().is_auditioning();

                if is_auditioning {
                    if styled_button(ui, "⏹ Stop", Color32::from_rgb(255, 80, 80)) {
                        state.engine.lock().unwrap().stop_audition();
                    }
                } else if styled_button(ui, "▶ Preview", ACCENT2) {
                    audition_path = Some(path.clone());
                }

                if styled_button(ui, "⚡ Load Into Slicer", ACCENT) {
                    load_path = Some(path);
                }
            });
            ui.separator();
        }

        // List of entries.
        egui::ScrollArea::vertical().show(ui, |ui| {
            let entries = state.file_browser.entries.clone();
            for entry in entries {
                let is_selected = state.file_browser.selected_file.as_ref() == Some(&entry.path);

                if entry.is_dir {
                    let text = format!("📁 {}", entry.name);
                    if ui.selectable_label(is_selected, text).double_clicked() {
                        let path = entry.path.clone();
                        state.file_browser.set_dir(path.clone());
                        *state.last_dir.lock().unwrap() = Some(path);
                        crate::sync_global_settings(&state.last_dir, &state.favorites);
                    }
                } else if entry.is_audio {
                    let ext = entry.path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                    let icon = if ext == "rx2" || ext == "rex" || ext == "rcy" { "🥁" } else { "🎵" };
                    let text = format!("{icon} {}", entry.name);

                    let resp = ui.selectable_label(is_selected, text);
                    if resp.double_clicked() {
                        audition_path = None;
                        load_path = Some(entry.path.clone());
                    } else if resp.clicked() {
                        state.file_browser.selected_file = Some(entry.path.clone());
                        if state.file_browser.auto_play {
                            audition_path = Some(entry.path.clone());
                        }
                    }
                }
            }
        });

        // Handle load request first so it can cancel auditioning.
        if let Some(path) = load_path {
            audition_path = None;
            load_file_into_slicer(&path, state);
        }

        // Handle audition request.
        if let Some(path) = audition_path {
            match audition_audio_file(&path, &state.engine) {
                Ok(()) => state.status(format!("Preview: {}", path.file_name().unwrap_or_default().to_string_lossy())),
                Err(e) => state.status(format!("Preview error: {e}")),
            }
        }
    });
}

fn audition_audio_file(path: &std::path::Path, engine: &Arc<Mutex<Engine>>) -> Result<(), String> {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    let pcm = match ext.as_str() {
        "rx2" | "rex" | "rcy" => SliceLoop::load_rex2(path)?.audio,
        _ => SliceLoop::load_audio(path)?.audio,
    };
    engine.lock().unwrap().play_audition(pcm);
    Ok(())
}

fn load_file_into_slicer(path: &std::path::Path, state: &mut EditorState) {
    // Immediately stop all auditioning, previewing, and active voices from previous sample
    state.engine.lock().unwrap().reset_playback();
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    if let Some(parent) = path.parent() {
        *state.last_dir.lock().unwrap() = Some(parent.to_path_buf());
        crate::sync_global_settings(&state.last_dir, &state.favorites);
    }
    match ext.as_str() {
        "rx2" | "rex" | "rcy" => {
            match SliceLoop::load_rex2(path) {
                Ok(sl) => {
                    let bpm = sl.bpm;
                    state.bpm_input = bpm;
                    *state.loop_data.write().unwrap() = Some(sl);
                    state.selected_slice = None;
                    state.transient_preview.clear();
                    state.status(format!("Loaded REX @ {bpm:.1} BPM"));
                }
                Err(e) => state.status(format!("Load REX error: {e}")),
            }
        }
        _ => {
            match SliceLoop::load_audio(path) {
                Ok(sl) => {
                    let bpm = sl.bpm;
                    state.bpm_input = bpm;
                    *state.loop_data.write().unwrap() = Some(sl);
                    state.selected_slice = None;
                    state.transient_preview.clear();
                    state.status(format!("Loaded Audio ({}) @ {bpm:.1} BPM: {}", ext.to_uppercase(), path.file_name().unwrap_or_default().to_string_lossy()));
                }
                Err(e) => state.status(format!("Load audio error: {e}")),
            }
        }
    }
}

fn handle_drag_and_drop(ui: &mut Ui, state: &mut EditorState) {
    let dropped = ui.input(|i| i.raw.dropped_files.clone());
    for file in dropped {
        if let Some(path) = file.path {
            load_file_into_slicer(&path, state);
        }
    }

    // Draw visual drop zone overlay when a file is hovering over the window.
    let is_hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
    if is_hovering {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(20, 50, 90, 200));
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(3.0, ACCENT), egui::StrokeKind::Outside);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "📥 Drop Audio File (WAV, AIFF, FLAC, MP3, M4A, OPUS, REX) here",
            FontId::new(22.0, FontFamily::Proportional),
            Color32::WHITE,
        );
    }
}

// ── Toolbar & Header Bar ──────────────────────────────────────────────────────
fn draw_toolbar(ui: &mut Ui, state: &mut EditorState) {
    // ── Row 1: Logo + Transport + Main Actions ────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("🎵 SlicePlayer")
            .font(FontId::new(17.0, FontFamily::Proportional))
            .color(ACCENT));

        ui.add_space(8.0);

        // PLAY / STOP
        let is_playing = state.engine.lock().unwrap().is_previewing();
        let (play_label, play_color) = if is_playing {
            ("⏹ STOP", Color32::from_rgb(255, 80, 80))
        } else {
            ("▶ PLAY", Color32::from_rgb(60, 200, 140))
        };

        if styled_button(ui, play_label, play_color) {
            let started = {
                let mut eng = state.engine.lock().unwrap();
                if is_playing {
                    eng.stop_preview();
                    false
                } else {
                    eng.play_preview();
                    true
                }
            };
            if started {
                state.status("Playing loop preview...");
            } else {
                state.status("Stopped preview.");
            }
        }

        ui.add_space(8.0);

        if styled_button(ui, "📥 Load File", ACCENT) {
            let mut dialog = rfd::FileDialog::new().add_filter(
                "Audio & Loop Files",
                &["wav", "aif", "aiff", "flac", "mp3", "m4a", "aac", "opus", "ogg", "rx2", "rex", "rcy"]
            );
            if let Some(dir) = state.last_dir.lock().unwrap().as_ref() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(path) = dialog.pick_file() {
                load_file_into_slicer(&path, state);
            }
        }

        ui.add_space(6.0);

        if styled_button(ui, "📂 Load Preset", Color32::from_rgb(180, 100, 220)) {
            let mut dialog = rfd::FileDialog::new().add_filter("SlicePlayer Preset", &["sliceplayer", "json"]);
            if let Some(dir) = state.last_dir.lock().unwrap().as_ref() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(path) = dialog.pick_file() {
                match crate::preset::load_preset_from_file(&path) {
                    Ok(sl) => {
                        let bpm = sl.bpm;
                        state.bpm_input = bpm;
                        *state.loop_data.write().unwrap() = Some(sl);
                        state.selected_slice = None;
                        state.transient_preview.clear();
                        state.status(format!("Loaded Preset: {}", path.file_name().unwrap_or_default().to_string_lossy()));
                    }
                    Err(e) => state.status(format!("Preset load error: {e}")),
                }
            }
        }

        if styled_button(ui, "💾 Save Preset", Color32::from_rgb(180, 100, 220)) {
            let mut dialog = rfd::FileDialog::new().add_filter("SlicePlayer Preset", &["sliceplayer", "json"]);
            if let Some(dir) = state.last_dir.lock().unwrap().as_ref() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(mut path) = dialog.save_file() {
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                if ext != "sliceplayer" && ext != "json" {
                    path.set_extension("sliceplayer");
                }
                let res = {
                    let guard = state.loop_data.read().unwrap();
                    if let Some(sl) = guard.as_ref() {
                        crate::preset::save_preset_to_file(&path, sl)
                    } else {
                        Err("No loop loaded to save as preset.".to_string())
                    }
                };
                match res {
                    Ok(()) => state.status(format!("Saved Preset: {}", path.file_name().unwrap_or_default().to_string_lossy())),
                    Err(e) => state.status(format!("Preset save error: {e}")),
                }
            }
        }

        ui.add_space(8.0);

        // Export Actions
        if styled_button(ui, "💾 Export WAV", ACCENT) {
            let mut dialog = rfd::FileDialog::new().add_filter("WAV Audio with Slices", &["wav"]);
            if let Some(dir) = state.last_dir.lock().unwrap().as_ref() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(path) = dialog.save_file() {
                let msg = {
                    let guard = state.loop_data.read().unwrap();
                    if let Some(sl) = guard.as_ref() {
                        match crate::exporter::export_sliced_wav(sl, &path) {
                            Ok(()) => format!("Exported Sliced WAV (Cue/Acid): {}", path.file_name().unwrap_or_default().to_string_lossy()),
                            Err(e) => format!("Export WAV Error: {e}"),
                        }
                    } else {
                        "No loop loaded.".to_string()
                    }
                };
                state.status(msg);
            }
        }

        if styled_button(ui, "📦 Export Multisample", ACCENT2) {
            let mut dialog = rfd::FileDialog::new().add_filter("Bitwig Multisample Archive", &["multisample"]);
            if let Some(dir) = state.last_dir.lock().unwrap().as_ref() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(mut path) = dialog.save_file() {
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                if ext != "multisample" {
                    path.set_extension("multisample");
                }
                let msg = {
                    let guard = state.loop_data.read().unwrap();
                    if let Some(sl) = guard.as_ref() {
                        match crate::exporter::export_bitwig_multisample(sl, &path) {
                            Ok(()) => {
                                let midi_name = path.with_extension("mid")
                                    .file_name().unwrap_or_default().to_string_lossy().into_owned();
                                format!("Exported: {} & {}", 
                                    path.file_name().unwrap_or_default().to_string_lossy(),
                                    midi_name)
                            },
                            Err(e) => format!("Export Multisample Error: {e}"),
                        }
                    } else {
                        "No loop loaded.".to_string()
                    }
                };
                state.status(msg);
            }
        }

        if styled_button(ui, "🎹 Export MIDI", ACCENT2) {
            let mut dialog = rfd::FileDialog::new().add_filter("MIDI File", &["mid", "midi"]);
            if let Some(dir) = state.last_dir.lock().unwrap().as_ref() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(mut path) = dialog.save_file() {
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                if ext != "mid" && ext != "midi" {
                    path.set_extension("mid");
                }
                let msg = {
                    let guard = state.loop_data.read().unwrap();
                    if let Some(sl) = guard.as_ref() {
                        match crate::midi_export::export_midi(sl, &path) {
                            Ok(()) => format!("Exported MIDI file: {}", path.file_name().unwrap_or_default().to_string_lossy()),
                            Err(e) => format!("Export MIDI Error: {e}"),
                        }
                    } else {
                        "No loop loaded.".to_string()
                    }
                };
                state.status(msg);
            }
        }

        let temp_midi_path = PathBuf::from("/tmp/slice_player_latest.mid");
        let btn_resp = ui.add(egui::Button::new(
            egui::RichText::new("🎹 Copy MIDI").color(ACCENT2).strong()
        ).fill(PANEL_BG).stroke(Stroke::new(1.0, ACCENT2.gamma_multiply(0.6))));

        if btn_resp.clicked() || btn_resp.drag_started() {
            let msg = {
                let guard = state.loop_data.read().unwrap();
                if let Some(sl) = guard.as_ref() {
                    if let Ok(()) = export_midi(sl, &temp_midi_path) {
                        let _ = copy_midi_file_to_clipboard(&temp_midi_path);
                        let uri = format!("file://{}", temp_midi_path.to_string_lossy());
                        ui.ctx().copy_text(uri);
                        "MIDI clip set to system clipboard! Press Ctrl+V in Bitwig.".to_string()
                    } else {
                        "Failed to generate temp MIDI clip.".to_string()
                    }
                } else {
                    "No loop loaded.".to_string()
                }
            };
            state.status(msg);
        }
        if btn_resp.dragged() {
            btn_resp.dnd_set_drag_payload(temp_midi_path.clone());
        }
    });

    ui.add_space(2.0);

    // ── Row 2: Sub Header (Browser Toggle, BPM, Bars, File Stats) ─────────────
    ui.horizontal(|ui| {
        let browser_col = if state.file_browser.visible { ACCENT } else { TEXT_DIM };
        if styled_button(ui, "📁 Browser", browser_col) {
            state.file_browser.visible = !state.file_browser.visible;
        }

        ui.add_space(8.0);

        ui.label(egui::RichText::new("BPM:").color(ACCENT));
        ui.add(egui::DragValue::new(&mut state.bpm_input)
            .speed(0.1).range(20.0..=300.0).fixed_decimals(1));

        ui.add_space(8.0);

        ui.label(egui::RichText::new("Bars:").color(ACCENT));
        let mut bars_val = {
            let guard = state.loop_data.read().unwrap();
            guard.as_ref().map(|sl| sl.calculate_bars()).unwrap_or(1.0)
        };
        let orig_bars = bars_val;
        for b in [1.0, 2.0, 4.0, 8.0, 16.0] {
            let is_selected = (bars_val - b).abs() < 0.05;
            let col = if is_selected { ACCENT } else { TEXT_DIM };
            let label = format!("{b:.0}");
            if ui.add(egui::SelectableLabel::new(is_selected, egui::RichText::new(label).color(col))).clicked() {
                bars_val = b;
            }
        }

        ui.add_space(6.0);
        if styled_button(ui, "🔄 Reset Loop", TEXT_DIM) {
            {
                let mut guard = state.loop_data.write().unwrap();
                if let Some(sl) = guard.as_mut() {
                    sl.loop_start = 0;
                    sl.loop_end = sl.total_frames;
                }
            }
            state.status("Loop range reset to full audio.");
        }

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Zoom:").color(ACCENT));
        ui.add(egui::Slider::new(&mut state.zoom_factor, 1.0..=20.0).suffix("x").fixed_decimals(1));
        if styled_button(ui, "🔍 100%", TEXT_DIM) {
            state.zoom_factor = 1.0;
            state.zoom_scroll = 0.0;
        }
        let status_msg = if (bars_val - orig_bars).abs() > 0.01 {
            let mut guard = state.loop_data.write().unwrap();
            if let Some(sl) = guard.as_mut() {
                sl.update_bpm_from_bars(bars_val);
                state.bpm_input = sl.bpm;
                Some(format!("Loop length: {bars_val:.1} Bars → Calculated BPM: {:.1}", sl.bpm))
            } else { None }
        } else { None };

        if let Some(m) = status_msg {
            state.status(m);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let guard = state.loop_data.read().unwrap();
            if let Some(sl) = guard.as_ref() {
                ui.label(egui::RichText::new(format!(
                    "{} slices  |  {:.1} BPM  |  {} Hz",
                    sl.slices.len(), sl.bpm, sl.sample_rate
                )).color(TEXT_DIM));
            }
        });
    });
}

// ── Waveform view ─────────────────────────────────────────────────────────────
fn draw_waveform(ui: &mut Ui, state: &mut EditorState) {
    let available = ui.available_width();
    let height = (ui.available_height() - 6.0).max(160.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(available, height), Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    // Background.
    painter.rect_filled(rect, 4.0, PANEL_BG);

    let guard = state.loop_data.read().unwrap();
    let Some(sl) = guard.as_ref() else {
        painter.text(
            rect.center(), Align2::CENTER_CENTER,
            "Drop a WAV or REX2 file here, or use Load buttons above.",
            FontId::proportional(14.0), TEXT_DIM,
        );
        return;
    };

    let total = sl.total_frames as f32;
    if total == 0.0 { return; }

    // ── Zooming & Scrolling Calculations ─────────────────────────────────────
    let zoom = state.zoom_factor.clamp(1.0, 30.0);
    let view_frames = total / zoom;
    let max_scroll = (1.0 - 1.0 / zoom).max(0.0);
    state.zoom_scroll = state.zoom_scroll.clamp(0.0, max_scroll);
    let view_start_f = state.zoom_scroll * total;

    let frame_to_x = |frame: f32| -> f32 {
        rect.left() + ((frame - view_start_f) / view_frames) * rect.width()
    };

    let x_to_frame = |x: f32| -> usize {
        (view_start_f + ((x - rect.left()) / rect.width()).clamp(0.0, 1.0) * view_frames) as usize
    };

    // Handle mouse wheel zoom & scroll on waveform view
    if response.hovered() {
        let scroll_delta = ui.input(|i| i.raw_scroll_delta);
        if scroll_delta.y != 0.0 {
            let zoom_change = 1.0 + scroll_delta.y * 0.002;
            let old_zoom = state.zoom_factor;
            let new_zoom = (old_zoom * zoom_change).clamp(1.0, 30.0);

            if let Some(pos) = response.hover_pos() {
                let mouse_frac = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                let mouse_frame = view_start_f + mouse_frac * view_frames;
                let new_view_frames = total / new_zoom;
                let new_view_start = mouse_frame - mouse_frac * new_view_frames;
                state.zoom_scroll = (new_view_start / total).clamp(0.0, (1.0 - 1.0 / new_zoom).max(0.0));
            }
            state.zoom_factor = new_zoom;
            ui.ctx().request_repaint();
        }

        if scroll_delta.x != 0.0 {
            let pan_delta = (scroll_delta.x / rect.width()) / zoom;
            state.zoom_scroll = (state.zoom_scroll - pan_delta).clamp(0.0, max_scroll);
            ui.ctx().request_repaint();
        }
    }

    // ── Waveform peaks ────────────────────────────────────────────────────────
    let buckets = sl.peak_cache.len().max(1);
    let mid_y = rect.center().y;
    let half = rect.height() * 0.5;

    for (i, &(neg, pos)) in sl.peak_cache.iter().enumerate() {
        let f0 = (i as f32 / buckets as f32) * total;
        let f1 = ((i + 1) as f32 / buckets as f32) * total;

        let x0 = frame_to_x(f0);
        let x1 = frame_to_x(f1);

        if x1 >= rect.left() && x0 <= rect.right() {
            let top    = mid_y - pos.abs().min(1.0) * half * 0.92;
            let bottom = mid_y + neg.abs().min(1.0) * half * 0.92;
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(x0.max(rect.left()), top), Pos2::new(x1.min(rect.right()).max(x0 + 1.0), bottom)),
                0.0, WAVEFORM.gamma_multiply(0.75),
            );
        }
    }

    // ── Dim region outside loop bounds ───────────────────────────────────────
    let x_loop_start = frame_to_x(sl.loop_start as f32);
    let x_loop_end   = frame_to_x(sl.loop_end as f32);

    if sl.loop_start > 0 && x_loop_start > rect.left() {
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(rect.left(), rect.top()), Pos2::new(x_loop_start.min(rect.right()), rect.bottom())),
            0.0,
            Color32::from_rgba_unmultiplied(0, 0, 0, 150),
        );
    }
    if sl.loop_end < sl.total_frames && x_loop_end < rect.right() {
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(x_loop_end.max(rect.left()), rect.top()), Pos2::new(rect.right(), rect.bottom())),
            0.0,
            Color32::from_rgba_unmultiplied(0, 0, 0, 150),
        );
    }

    // ── 1/16th Beat Grid Overlay inside [loop_start..loop_end] ───────────────
    let loop_start_f = sl.loop_start;
    let loop_end_f = sl.loop_end.min(sl.total_frames);
    let loop_frames = loop_end_f.saturating_sub(loop_start_f);

    if loop_frames > 0 {
        let bars = sl.calculate_bars();
        let total_16ths = (bars * 16.0).round().max(1.0) as usize;
        let step_frames = loop_frames as f64 / total_16ths as f64;

        for i in 0..=total_16ths {
            let f_pos = loop_start_f + (i as f64 * step_frames) as usize;
            if f_pos > sl.total_frames { break; }
            let x = frame_to_x(f_pos as f32);
            if x < rect.left() || x > rect.right() { continue; }

            let (stroke_w, line_col, label) = if i % 16 == 0 {
                let bar_num = (i / 16) + 1;
                (1.5, Color32::from_rgba_unmultiplied(255, 255, 255, 140), Some(format!("{bar_num}.1")))
            } else if i % 4 == 0 {
                let _beat_num = (i / 4) % 4 + 1;
                (1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 75), None)
            } else {
                (0.6, Color32::from_rgba_unmultiplied(255, 255, 255, 30), None)
            };

            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(stroke_w, line_col),
            );

            if let Some(lbl) = label {
                painter.text(
                    Pos2::new(x + 2.0, rect.bottom() - 14.0),
                    Align2::LEFT_BOTTOM,
                    lbl,
                    FontId::monospace(9.0),
                    line_col,
                );
            }
        }
    }

    // ── Green Loop Start [S] & Red Loop End [E] Markers ──────────────────────
    let col_s = Color32::from_rgb(50, 220, 100);
    painter.line_segment([Pos2::new(x_loop_start, rect.top()), Pos2::new(x_loop_start, rect.bottom())], Stroke::new(2.5, col_s));
    let flag_s = vec![
        Pos2::new(x_loop_start, rect.top()),
        Pos2::new(x_loop_start + 16.0, rect.top()),
        Pos2::new(x_loop_start + 16.0, rect.top() + 14.0),
        Pos2::new(x_loop_start, rect.top() + 14.0),
    ];
    painter.add(egui::Shape::convex_polygon(flag_s, col_s, Stroke::NONE));
    painter.text(Pos2::new(x_loop_start + 8.0, rect.top() + 7.0), Align2::CENTER_CENTER, "S", FontId::monospace(10.0), Color32::BLACK);

    let col_e = Color32::from_rgb(255, 70, 70);
    painter.line_segment([Pos2::new(x_loop_end, rect.top()), Pos2::new(x_loop_end, rect.bottom())], Stroke::new(2.5, col_e));
    let flag_e = vec![
        Pos2::new(x_loop_end - 16.0, rect.top()),
        Pos2::new(x_loop_end, rect.top()),
        Pos2::new(x_loop_end, rect.top() + 14.0),
        Pos2::new(x_loop_end - 16.0, rect.top() + 14.0),
    ];
    painter.add(egui::Shape::convex_polygon(flag_e, col_e, Stroke::NONE));
    painter.text(Pos2::new(x_loop_end - 8.0, rect.top() + 7.0), Align2::CENTER_CENTER, "E", FontId::monospace(10.0), Color32::WHITE);

    // ── Slice markers & Fade Envelopes ────────────────────────────────────────
    let pointer = response.hover_pos();
    let mut hover_marker: Option<usize> = None;
    let mut hover_fade: Option<(usize, bool)> = None;
    let mut hover_loop: Option<bool> = None;

    // Check hover state for loop start/end and fade handles
    if let Some(pos) = pointer {
        let x_s = rect.left() + (sl.loop_start as f32 / total) * rect.width();
        if (pos.x - x_s).abs() < 12.0 {
            hover_loop = Some(false);
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        let x_e = rect.left() + (sl.loop_end as f32 / total) * rect.width();
        if hover_loop.is_none() && (pos.x - x_e).abs() < 12.0 {
            hover_loop = Some(true);
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }

        if hover_loop.is_none() {
            for (idx, slice) in sl.slices.iter().enumerate() {
                let fade_out_f = slice.fade_out_frames(sl.sample_rate);
                let x_fade_out = rect.left() + (slice.end.saturating_sub(fade_out_f) as f32 / total) * rect.width();
                let handle_out = Pos2::new(x_fade_out, rect.top() + 8.0);
                if (pos - handle_out).length() < 9.0 {
                    hover_fade = Some((idx, true));
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    break;
                }

                let fade_in_f = slice.fade_in_frames(sl.sample_rate);
                let x_fade_in = rect.left() + ((slice.start + fade_in_f) as f32 / total) * rect.width();
                let handle_in = Pos2::new(x_fade_in, rect.top() + 8.0);
                if (pos - handle_in).length() < 9.0 {
                    hover_fade = Some((idx, false));
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    break;
                }
            }
        }

        if hover_loop.is_none() && hover_fade.is_none() {
            for (idx, slice) in sl.slices.iter().enumerate().skip(1) {
                let x = rect.left() + (slice.start as f32 / total) * rect.width();
                if (pos.x - x).abs() < MARKER_GRAB_PX {
                    hover_marker = Some(idx);
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    break;
                }
            }
        }
    }

    // ── Render Selected Slice Highlight Overlay ─────────────────────────────────────────
    if let Some(sel_idx) = state.selected_slice {
        if let Some(slice) = sl.slices.get(sel_idx) {
            let x_start = frame_to_x(slice.start as f32);
            let x_end   = frame_to_x(slice.end as f32);
            let x0 = x_start.max(rect.left());
            let x1 = x_end.min(rect.right());
            if x1 > x0 {
                let sel_rect = Rect::from_min_max(Pos2::new(x0, rect.top()), Pos2::new(x1, rect.bottom()));
                // Vibrant translucent gold/orange fill overlay
                painter.rect_filled(
                    sel_rect,
                    0.0,
                    Color32::from_rgba_unmultiplied(255, 170, 40, 45),
                );
                // Glowing border box frame
                painter.rect_stroke(
                    sel_rect,
                    0.0,
                    Stroke::new(2.5, Color32::from_rgb(255, 170, 40)),
                    egui::StrokeKind::Inside,
                );
                // Header badge at top left of selected slice
                let badge_text = format!("Slice #{} ({})", sel_idx + 1, midi_note_name(slice.note));
                let badge_pos = Pos2::new(x0 + 4.0, rect.top() + 4.0);
                painter.rect_filled(
                    Rect::from_min_size(badge_pos, Vec2::new(95.0, 16.0)),
                    3.0,
                    Color32::from_rgba_unmultiplied(20, 24, 32, 220),
                );
                painter.text(
                    Pos2::new(badge_pos.x + 4.0, badge_pos.y + 2.0),
                    Align2::LEFT_TOP,
                    badge_text,
                    FontId::monospace(10.0),
                    Color32::from_rgb(255, 190, 60),
                );
            }
        }
    }

    // Render Fade Overlays & Handles for each slice
    for (idx, slice) in sl.slices.iter().enumerate() {
        let x_start = rect.left() + (slice.start as f32 / total) * rect.width();
        let x_end   = rect.left() + (slice.end   as f32 / total) * rect.width();
        let is_selected = state.selected_slice == Some(idx);

        let fade_in_f = slice.fade_in_frames(sl.sample_rate);
        let fade_out_f = slice.fade_out_frames(sl.sample_rate);

        // Fade In Envelope & Handle
        if fade_in_f > 0 || is_selected {
            let x_fade_in = rect.left() + ((slice.start + fade_in_f) as f32 / total) * rect.width();
            let polygon = vec![
                Pos2::new(x_start, rect.top()),
                Pos2::new(x_fade_in, rect.top()),
                Pos2::new(x_start, rect.bottom()),
            ];
            painter.add(egui::Shape::convex_polygon(
                polygon,
                Color32::from_rgba_unmultiplied(80, 220, 255, 30),
                Stroke::NONE,
            ));
            painter.line_segment(
                [Pos2::new(x_start, rect.bottom()), Pos2::new(x_fade_in, rect.top())],
                Stroke::new(1.5, Color32::from_rgb(80, 220, 255)),
            );

            let handle_pos = Pos2::new(x_fade_in, rect.top() + 8.0);
            let handle_col = if state.dragging_fade == Some((idx, false)) || hover_fade == Some((idx, false)) {
                Color32::WHITE
            } else {
                Color32::from_rgb(80, 220, 255)
            };
            painter.circle_filled(handle_pos, 4.0, handle_col);
        }

        // Fade Out Envelope & Handle
        if fade_out_f > 0 || is_selected {
            let x_fade_out = rect.left() + (slice.end.saturating_sub(fade_out_f) as f32 / total) * rect.width();
            let polygon = vec![
                Pos2::new(x_fade_out, rect.top()),
                Pos2::new(x_end, rect.top()),
                Pos2::new(x_end, rect.bottom()),
            ];
            painter.add(egui::Shape::convex_polygon(
                polygon,
                Color32::from_rgba_unmultiplied(255, 200, 80, 30),
                Stroke::NONE,
            ));
            painter.line_segment(
                [Pos2::new(x_fade_out, rect.top()), Pos2::new(x_end, rect.bottom())],
                Stroke::new(1.5, Color32::from_rgb(255, 200, 80)),
            );

            let handle_pos = Pos2::new(x_fade_out, rect.top() + 8.0);
            let handle_col = if state.dragging_fade == Some((idx, true)) || hover_fade == Some((idx, true)) {
                Color32::WHITE
            } else {
                Color32::from_rgb(255, 200, 80)
            };
            painter.circle_filled(handle_pos, 4.0, handle_col);
        }
    }

    for (idx, slice) in sl.slices.iter().enumerate() {
        if idx == 0 { continue; } // Don't draw marker at start
        let x = rect.left() + (slice.start as f32 / total) * rect.width();
        let col = if state.dragging_marker == Some(idx) || hover_marker == Some(idx) {
            MARKER_HO
        } else {
            MARKER
        };
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(2.0, col),
        );
        // Note label.
        let note_name = midi_note_name(slice.note);
        painter.text(
            Pos2::new(x + 3.0, rect.top() + 4.0),
            Align2::LEFT_TOP, note_name,
            FontId::monospace(9.0), col,
        );
    }

    // ── Playhead cursor(s) ───────────────────────────────────────────────────
    let playheads = state.engine.lock().unwrap().active_playhead_frames(sl);
    for frame in &playheads {
        let x = rect.left() + (*frame as f32 / total) * rect.width();
        if x >= rect.left() && x <= rect.right() {
            // Glowing cyan cursor line
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(2.5, Color32::from_rgb(0, 255, 220)),
            );
            // Triangular cursor head marker at top of waveform
            let head_size = 5.0f32;
            let triangle = vec![
                Pos2::new(x - head_size, rect.top()),
                Pos2::new(x + head_size, rect.top()),
                Pos2::new(x, rect.top() + head_size * 1.5),
            ];
            painter.add(egui::Shape::convex_polygon(
                triangle,
                Color32::from_rgb(0, 255, 220),
                Stroke::NONE,
            ));
        }
    }

    // Request continuous UI repaint while audio is playing for smooth 60 FPS playhead movement.
    if !playheads.is_empty() {
        ui.ctx().request_repaint();
    }

    // ── Interaction ───────────────────────────────────────────────────────────
    drop(guard); // Release read lock before taking write lock.

    // Start drag on a loop marker, fade handle, or slice marker.
    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            let guard = state.loop_data.read().unwrap();
            if let Some(sl) = guard.as_ref() {
                let total = sl.total_frames as f32;

                let x_s = rect.left() + (sl.loop_start as f32 / total) * rect.width();
                let x_e = rect.left() + (sl.loop_end as f32 / total) * rect.width();

                if (pos.x - x_s).abs() < 14.0 {
                    state.dragging_loop_bound = Some(false);
                } else if (pos.x - x_e).abs() < 14.0 {
                    state.dragging_loop_bound = Some(true);
                } else {
                    // Check fade handles
                    let mut fade_hit = None;
                    for (idx, slice) in sl.slices.iter().enumerate() {
                        let fade_out_f = slice.fade_out_frames(sl.sample_rate);
                        let x_fade_out = rect.left() + (slice.end.saturating_sub(fade_out_f) as f32 / total) * rect.width();
                        let handle_out = Pos2::new(x_fade_out, rect.top() + 8.0);
                        if (pos - handle_out).length() < 10.0 {
                            fade_hit = Some((idx, true));
                            break;
                        }

                        let fade_in_f = slice.fade_in_frames(sl.sample_rate);
                        let x_fade_in = rect.left() + ((slice.start + fade_in_f) as f32 / total) * rect.width();
                        let handle_in = Pos2::new(x_fade_in, rect.top() + 8.0);
                        if (pos - handle_in).length() < 10.0 {
                            fade_hit = Some((idx, false));
                            break;
                        }
                    }

                    if let Some(hit) = fade_hit {
                        state.dragging_fade = Some(hit);
                        state.selected_slice = Some(hit.0);
                    } else {
                        // Check slice markers
                        for (idx, slice) in sl.slices.iter().enumerate().skip(1) {
                            let x = rect.left() + (slice.start as f32 / total) * rect.width();
                            if (pos.x - x).abs() < MARKER_GRAB_PX {
                                state.dragging_marker = Some(idx);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // Move loop start / end marker during drag.
    if let Some(is_loop_end) = state.dragging_loop_bound {
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let new_frame = x_to_frame(pos.x);
                let mut guard = state.loop_data.write().unwrap();
                if let Some(sl) = guard.as_mut() {
                    if is_loop_end {
                        sl.loop_end = new_frame.max(sl.loop_start + 100).min(sl.total_frames);
                    } else {
                        sl.loop_start = new_frame.min(sl.loop_end.saturating_sub(100));
                    }
                }
            }
        } else {
            state.dragging_loop_bound = None;
        }
    }

    // Move fade handle during drag.
    if let Some((drag_slice_idx, is_fade_out)) = state.dragging_fade {
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let mouse_frame = x_to_frame(pos.x);
                let mut guard = state.loop_data.write().unwrap();
                if let Some(sl) = guard.as_mut() {
                    let sample_rate = sl.sample_rate;
                    if let Some(slice) = sl.slices.get_mut(drag_slice_idx) {
                        let max_fade_frames = slice.frame_count() / 2;
                        if is_fade_out {
                            let fade_frames = slice.end.saturating_sub(mouse_frame);
                            let clamped_frames = fade_frames.min(max_fade_frames);
                            slice.fade_out_ms = clamped_frames as f32 * 1000.0 / sample_rate as f32;
                        } else {
                            let fade_frames = mouse_frame.saturating_sub(slice.start);
                            let clamped_frames = fade_frames.min(max_fade_frames);
                            slice.fade_in_ms = clamped_frames as f32 * 1000.0 / sample_rate as f32;
                        }
                    }
                }
            }
        } else {
            state.dragging_fade = None;
        }
    }

    // Move marker during drag.
    if let Some(drag_idx) = state.dragging_marker {
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let new_frame = x_to_frame(pos.x);
                let mut guard = state.loop_data.write().unwrap();
                if let Some(sl) = guard.as_mut() {
                    sl.move_slice_start(drag_idx, new_frame);
                    sl.rebuild_peaks(1024);
                }
            }
        } else {
            state.dragging_marker = None;
        }
    }

    // Left-click (not on a marker): select slice or insert new marker.
    if response.clicked() && state.dragging_marker.is_none() && state.dragging_loop_bound.is_none() && state.dragging_fade.is_none() {
        if let Some(pos) = response.interact_pointer_pos() {
            let click_frame = x_to_frame(pos.x);
            let mut guard = state.loop_data.write().unwrap();
            if let Some(sl) = guard.as_mut() {
                // Check if we're near an existing marker.
                let near_marker = sl.slices.iter().enumerate().skip(1).find(|(_, s)| {
                    let mx = frame_to_x(s.start as f32);
                    (pos.x - mx).abs() < MARKER_GRAB_PX
                });

                if let Some((idx, _)) = near_marker {
                    state.selected_slice = Some(idx);
                } else {
                    // Which slice did we click in?
                    let clicked_slice = sl.slices.iter().position(|s| s.start <= click_frame && click_frame < s.end);
                    state.selected_slice = clicked_slice;

                    // Shift+Click or Double-Click inserts a new slice marker.
                    if ui.input(|i| i.modifiers.shift) || response.double_clicked() {
                        let msg = {
                            sl.insert_slice_at(click_frame);
                            sl.rebuild_peaks(1024);
                            format!("Inserted slice at frame {click_frame}")
                        };
                        drop(guard);
                        state.status(msg);
                    }
                }
            }
        }
    }

    // Right-click: delete marker (if near one).
    if response.secondary_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let msg = {
                let mut guard = state.loop_data.write().unwrap();
                if let Some(sl) = guard.as_mut() {
                    let total = sl.total_frames as f32;
                    if let Some((idx, _)) = sl.slices.iter().enumerate().skip(1).find(|(_, s)| {
                        let mx = rect.left() + (s.start as f32 / total) * rect.width();
                        (pos.x - mx).abs() < MARKER_GRAB_PX * 2.0
                    }) {
                        sl.remove_slice(idx);
                        sl.rebuild_peaks(1024);
                        if let Some(sel) = state.selected_slice {
                            if sel >= sl.slices.len() { state.selected_slice = None; }
                        }
                        Some(format!("Removed slice #{idx}"))
                    } else { None }
                } else { None }
            };
            if let Some(m) = msg {
                state.engine.lock().unwrap().reset_playback();
                state.status(m);
            }
        }
    }

    // Interaction hint label.
    ui.label(egui::RichText::new(
        "Doppelklick / Shift+Click: Slice einfügen  |  Rechtsklick: Entfernen  |  Drag: Verschieben"
    ).color(TEXT_DIM).size(10.0));
}

// ── Per-slice parameter editor ────────────────────────────────────────────────
fn draw_slice_editor(ui: &mut Ui, state: &mut EditorState) {
    let Some(sel) = state.selected_slice else { return; };

    let mut delete_requested = false;
    let mut copy_to_all_requested = false;
    let mut reset_fx_requested = false;

    {
        let mut guard = state.loop_data.write().unwrap();
        let Some(sl) = guard.as_mut() else { return; };
        if sel >= sl.slices.len() { state.selected_slice = None; return; }
        let slice = &mut sl.slices[sel];

        egui::Frame::NONE
            .fill(PANEL_BG)
            .inner_margin(6.0)
            .corner_radius(6.0)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("Slice #{} ({})", sel + 1, midi_note_name(slice.note)))
                            .color(ACCENT).strong());

                        ui.add_space(8.0);
                        ui.label("Note:");
                        egui::ComboBox::new("slice_note", "")
                            .selected_text(midi_note_name(slice.note))
                            .show_ui(ui, |ui| {
                                for n in 0u8..=127 {
                                    ui.selectable_value(&mut slice.note, n, midi_note_name(n));
                                }
                            });

                        ui.add_space(8.0);
                        ui.label("Gain:");
                        ui.add(egui::Slider::new(&mut slice.gain, 0.0..=2.0).suffix("x").fixed_decimals(2));

                        ui.add_space(8.0);
                        ui.label("Pan:");
                        ui.add(egui::Slider::new(&mut slice.pan, -1.0..=1.0).fixed_decimals(2));

                        ui.add_space(8.0);
                        ui.label("Pitch:");
                        ui.add(egui::Slider::new(&mut slice.pitch_semitones, -24.0..=24.0)
                            .suffix(" st").fixed_decimals(1));

                        ui.add_space(8.0);
                        ui.label("Fade In:");
                        let max_in_ms = (slice.frame_count() / 2) as f32 * 1000.0 / sl.sample_rate as f32;
                        ui.add(egui::Slider::new(&mut slice.fade_in_ms, 0.0..=max_in_ms.max(10.0)).suffix(" ms").fixed_decimals(1));

                        ui.add_space(8.0);
                        ui.label("Fade Out:");
                        let max_out_ms = (slice.frame_count() / 2) as f32 * 1000.0 / sl.sample_rate as f32;
                        ui.add(egui::Slider::new(&mut slice.fade_out_ms, 0.0..=max_out_ms.max(10.0)).suffix(" ms").fixed_decimals(1));

                        ui.add_space(6.0);
                        ui.checkbox(&mut slice.reverse, "Rev");

                        ui.add_space(6.0);
                        ui.checkbox(&mut slice.muted, "Mute");

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if styled_button(ui, "🗑️ Delete", Color32::from_rgb(220, 70, 70)) {
                                delete_requested = true;
                            }
                        });
                    });

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Row 2: Per-Slice DSP / FX Engine Controls
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("🎛️ Slice FX:").color(ACCENT).strong());

                        // DJM Mixer-Style Lowpass / Highpass Combo Filter
                        ui.add_space(4.0);
                        ui.label("🎚️ DJM Filter:");
                        ui.add(egui::Slider::new(&mut slice.fx.filter_djm, -1.0..=1.0)
                            .custom_formatter(|val, _| {
                                if val < -0.01 {
                                    let norm = (1.0 + val.clamp(-1.0, 0.0)) as f32;
                                    let hz = 20.0f32 * (20000.0f32 / 20.0f32).powf(norm);
                                    format!("LP {:.0} Hz", hz)
                                } else if val > 0.01 {
                                    let norm = val.clamp(0.0, 1.0) as f32;
                                    let hz = 20.0f32 * (15000.0f32 / 20.0f32).powf(norm);
                                    format!("HP {:.0} Hz", hz)
                                } else {
                                    "Neutral (Off)".to_string()
                                }
                            }));

                        ui.label("Res:");
                        ui.add(egui::Slider::new(&mut slice.fx.filter_resonance, 0.5..=10.0)
                            .suffix(" Q").fixed_decimals(2));

                        // Bitcrusher
                        ui.add_space(6.0);
                        ui.label("Crush:");
                        ui.add(egui::Slider::new(&mut slice.fx.bit_depth, 1.0..=16.0)
                            .suffix(" bits").fixed_decimals(1));

                        ui.label("Downsample:");
                        ui.add(egui::Slider::new(&mut slice.fx.downsample_factor, 1..=32)
                            .suffix("x"));

                        // Drive
                        ui.add_space(6.0);
                        ui.label("Drive:");
                        ui.add(egui::Slider::new(&mut slice.fx.drive, 0.0..=1.0)
                            .fixed_decimals(2));

                        // Retrigger
                        ui.add_space(6.0);
                        ui.label("Retrig:");
                        egui::ComboBox::new("retrigger_rate", "")
                            .selected_text(slice.fx.retrigger_rate.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut slice.fx.retrigger_rate, RetriggerRate::Off, "Off");
                                ui.selectable_value(&mut slice.fx.retrigger_rate, RetriggerRate::Eighth, "1/8");
                                ui.selectable_value(&mut slice.fx.retrigger_rate, RetriggerRate::Sixteenth, "1/16");
                                ui.selectable_value(&mut slice.fx.retrigger_rate, RetriggerRate::ThirtySecond, "1/32");
                                ui.selectable_value(&mut slice.fx.retrigger_rate, RetriggerRate::SixtyFourth, "1/64");
                            });

                        if slice.fx.retrigger_rate != RetriggerRate::Off {
                            ui.label("Decay:");
                            ui.add(egui::Slider::new(&mut slice.fx.retrigger_decay, 0.0..=1.0)
                                .fixed_decimals(2));
                        }

                        // Choke Group
                        ui.add_space(6.0);
                        ui.label("Choke:");
                        egui::ComboBox::new("choke_group", "")
                            .selected_text(if slice.fx.choke_group == 0 { "Off".to_string() } else { format!("Grp {}", slice.fx.choke_group) })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut slice.fx.choke_group, 0, "Off");
                                for g in 1u8..=8 {
                                    ui.selectable_value(&mut slice.fx.choke_group, g, format!("Group {g}"));
                                }
                            });
                    });

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Row 3: Akai Vintage Timestretch & DJM500 Oldschool Jungle Dub Echo
                    ui.horizontal(|ui| {
                        // Akai Timestretch
                        ui.label(egui::RichText::new("📼 Akai Stretch:").color(ACCENT2).strong());
                        ui.add(egui::Slider::new(&mut slice.fx.stretch_factor, 0.5..=2.0)
                            .suffix("x").fixed_decimals(2));

                        if (slice.fx.stretch_factor - 1.0).abs() > 0.01 {
                            ui.label("Grain:");
                            ui.add(egui::Slider::new(&mut slice.fx.stretch_grain_ms, 10.0..=100.0)
                                .suffix(" ms").fixed_decimals(0));
                        }

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        // DJM500 Dub Echo
                        ui.label(egui::RichText::new("📻 Jungle Dub Echo:").color(ACCENT2).strong());
                        ui.label("Time:");
                        egui::ComboBox::new("delay_rate", "")
                            .selected_text(slice.fx.delay_rate.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut slice.fx.delay_rate, DelayRate::Off, "Off");
                                ui.selectable_value(&mut slice.fx.delay_rate, DelayRate::Ms, "Free ms");
                                ui.selectable_value(&mut slice.fx.delay_rate, DelayRate::SixtyFourth, "1/64");
                                ui.selectable_value(&mut slice.fx.delay_rate, DelayRate::ThirtySecond, "1/32");
                                ui.selectable_value(&mut slice.fx.delay_rate, DelayRate::Sixteenth, "1/16");
                                ui.selectable_value(&mut slice.fx.delay_rate, DelayRate::DottedSixteenth, "1/16 Dotted");
                                ui.selectable_value(&mut slice.fx.delay_rate, DelayRate::Eighth, "1/8");
                                ui.selectable_value(&mut slice.fx.delay_rate, DelayRate::DottedEighth, "3/16 Dotted");
                                ui.selectable_value(&mut slice.fx.delay_rate, DelayRate::Quarter, "1/4");
                                ui.selectable_value(&mut slice.fx.delay_rate, DelayRate::Half, "1/2");
                            });

                        if slice.fx.delay_rate == DelayRate::Ms {
                            ui.label("ms:");
                            ui.add(egui::Slider::new(&mut slice.fx.delay_ms, 1.0..=100.0)
                                .suffix(" ms").fixed_decimals(1));
                        }

                        if slice.fx.delay_rate != DelayRate::Off {
                            ui.label("FB:");
                            ui.add(egui::Slider::new(&mut slice.fx.delay_feedback, 0.0..=0.90)
                                .fixed_decimals(2));

                            ui.label("Mix:");
                            ui.add(egui::Slider::new(&mut slice.fx.delay_mix, 0.0..=1.0)
                                .fixed_decimals(2));

                            ui.label("Tone (LP):");
                            ui.add(egui::Slider::new(&mut slice.fx.delay_tone, 200.0..=12000.0)
                                .logarithmic(true).suffix(" Hz").fixed_decimals(0));
                        }
                    });

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Row 4: Batch Action Buttons
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("⚡ Batch Actions:").color(ACCENT).strong());
                        ui.add_space(6.0);
                        if styled_button(ui, "📋 Copy FX to All Slices", Color32::from_rgb(60, 160, 220)) {
                            copy_to_all_requested = true;
                        }
                        ui.add_space(8.0);
                        if styled_button(ui, "🔄 Reset Slice FX", Color32::from_rgb(180, 120, 60)) {
                            reset_fx_requested = true;
                        }
                    });
                });
            });
    }

    if copy_to_all_requested {
        {
            let mut guard = state.loop_data.write().unwrap();
            if let Some(sl) = guard.as_mut() {
                sl.copy_slice_fx_to_all(sel);
            }
        }
        state.status(format!("Copied FX settings from Slice #{} to all slices", sel + 1));
    }

    if reset_fx_requested {
        {
            let mut guard = state.loop_data.write().unwrap();
            if let Some(sl) = guard.as_mut() {
                if let Some(slice) = sl.slices.get_mut(sel) {
                    slice.reset_fx();
                }
            }
        }
        state.status(format!("Reset FX settings for Slice #{}", sel + 1));
    }

    if delete_requested {
        state.engine.lock().unwrap().reset_playback();
        let msg = {
            let mut guard = state.loop_data.write().unwrap();
            if let Some(sl) = guard.as_mut() {
                if sel < sl.slices.len() {
                    sl.remove_slice(sel);
                    sl.rebuild_peaks(1024);
                    state.selected_slice = None;
                    Some(format!("Deleted slice #{}", sel + 1))
                } else { None }
            } else { None }
        };
        if let Some(m) = msg { state.status(m); }
    }
}

// ── Slice mode panel ─────────────────────────────────────────────────────────
fn draw_slice_mode_panel(ui: &mut Ui, state: &mut EditorState) {
    egui::Frame::NONE
        .fill(PANEL_BG)
        .inner_margin(6.0)
        .corner_radius(6.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Slicing Mode Tabs
                ui.label(egui::RichText::new("Slicing Mode:").color(TEXT_DIM));

                let is_trans = state.slicing_tab == SlicingModeTab::Transient;
                if ui.add(egui::SelectableLabel::new(is_trans, egui::RichText::new("⚡ Transients").color(if is_trans { ACCENT } else { TEXT_DIM }).strong())).clicked() {
                    state.slicing_tab = SlicingModeTab::Transient;
                }

                let is_grid = state.slicing_tab == SlicingModeTab::Grid;
                if ui.add(egui::SelectableLabel::new(is_grid, egui::RichText::new("📐 Grid").color(if is_grid { ACCENT } else { TEXT_DIM }).strong())).clicked() {
                    state.slicing_tab = SlicingModeTab::Grid;
                }

                let is_man = state.slicing_tab == SlicingModeTab::Manual;
                if ui.add(egui::SelectableLabel::new(is_man, egui::RichText::new("✂️ Manual").color(if is_man { ACCENT } else { TEXT_DIM }).strong())).clicked() {
                    state.slicing_tab = SlicingModeTab::Manual;
                }

                ui.separator();

                // Tab Content
                match state.slicing_tab {
                    SlicingModeTab::Transient => {
                        ui.label("Thresh:");
                        let r_th = ui.add(egui::Slider::new(&mut state.transient_settings.threshold, 0.05..=3.00).fixed_decimals(2));
                        ui.label("Gap ms:");
                        let r_gap = ui.add(egui::DragValue::new(&mut state.transient_settings.min_gap_ms).speed(1.0).range(10.0..=500.0).fixed_decimals(0));
                        ui.label("Pre-roll ms:");
                        let r_pre = ui.add(egui::DragValue::new(&mut state.transient_settings.pre_roll_ms).speed(0.5).range(0.0..=50.0).fixed_decimals(1));

                        let detect_clicked = styled_button(ui, "⚡ Re-Detect", ACCENT);

                        if r_th.changed() || r_gap.changed() || r_pre.changed() || detect_clicked {
                            let settings = state.transient_settings.clone();
                            let bpm = state.bpm_input;
                            let msg = {
                                let mut guard = state.loop_data.write().unwrap();
                                if let Some(sl) = guard.as_mut() {
                                    if let Ok(()) = sl.detect_transients(&settings, bpm) {
                                        sl.rebuild_peaks(1024);
                                        let n = sl.slices.len();
                                        state.selected_slice = None;
                                        state.transient_preview.clear();
                                        Some(format!("Detected {n} transients @ Thresh {:.2}", settings.threshold))
                                    } else { None }
                                } else { None }
                            };
                            if let Some(m) = msg { state.status(m); }
                        }
                    }
                    SlicingModeTab::Grid => {
                        ui.label("Grid Division:");
                        for div in [GridDivision::Quarter, GridDivision::Eighth, GridDivision::Sixteenth, GridDivision::ThirtySecond] {
                            let selected = state.grid_division == div;
                            let col = if selected { ACCENT } else { TEXT_DIM };
                            if ui.add(egui::SelectableLabel::new(selected, egui::RichText::new(div.label()).color(col))).clicked() {
                                state.grid_division = div;
                            }
                        }
                        if styled_button(ui, "📐 Apply Grid", ACCENT) {
                            state.engine.lock().unwrap().reset_playback();
                            let div = state.grid_division;
                            let bpm = state.bpm_input;
                            let msg = {
                                let mut guard = state.loop_data.write().unwrap();
                                if let Some(sl) = guard.as_mut() {
                                    sl.apply_grid(div, bpm);
                                    sl.rebuild_peaks(1024);
                                    state.selected_slice = None;
                                    Some(format!("Grid {} applied @ {bpm:.1} BPM", div.label()))
                                } else { None }
                            };
                            if let Some(m) = msg { state.status(m); }
                        }
                    }
                    SlicingModeTab::Manual => {
                        ui.label(egui::RichText::new("Doppelklick / Shift+Click: Slice  |  Rechtsklick: Löschen  |  Drag: Verschieben").color(TEXT_DIM).size(11.0));
                        ui.add_space(10.0);
                        if styled_button(ui, "🗑️ Clear All Slices", Color32::from_rgb(220, 70, 70)) {
                            state.engine.lock().unwrap().reset_playback();
                            let msg = {
                                let mut guard = state.loop_data.write().unwrap();
                                if let Some(sl) = guard.as_mut() {
                                    let total = sl.total_frames;
                                    sl.slices.clear();
                                    sl.slices.push(crate::slicer::Slice::new(0, total, 48));
                                    state.selected_slice = None;
                                    Some("All slices cleared.".to_string())
                                } else { None }
                            };
                            if let Some(m) = msg { state.status(m); }
                        }
                    }
                }
            });
        });
}

// ── Status bar ────────────────────────────────────────────────────────────────
fn draw_status(ui: &mut Ui, state: &EditorState) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&state.status_msg).color(TEXT_DIM).size(11.0));
    });
}

// ── Helpers ───────────────────────────────────────────────────────────────────
fn styled_button(ui: &mut Ui, label: &str, col: Color32) -> bool {
    ui.add(egui::Button::new(egui::RichText::new(label).color(col).strong())
        .fill(PANEL_BG)
        .stroke(Stroke::new(1.0, col.gamma_multiply(0.6)))).clicked()
}

/// Convert a MIDI note number (0–127) to e.g. "C3", "F#4".
pub fn midi_note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"];
    let name = NAMES[(note % 12) as usize];
    let octave = (note as i32 / 12) - 2;
    format!("{name}{octave}")
}
