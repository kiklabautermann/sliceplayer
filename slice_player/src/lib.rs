//! SlicePlayer CLAP/VST3 plugin — nih-plug entry point.

mod velocloops_ffi;
mod slicer;
mod engine;
mod midi_export;
mod exporter;
pub mod settings;
pub mod preset;
mod editor;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use nih_plug_egui::resizable_window::ResizableWindow;

use engine::Engine;
use slicer::SliceLoop;
use editor::EditorState;

fn default_cutoff() -> f32 { 20000.0 }
fn default_resonance() -> f32 { 0.707 }
fn default_bit_depth() -> f32 { 16.0 }
fn default_downsample() -> u32 { 1 }
fn default_stretch_factor() -> f32 { 1.0 }
fn default_stretch_grain_ms() -> f32 { 30.0 }
fn default_delay_feedback() -> f32 { 0.45 }
fn default_delay_mix() -> f32 { 0.35 }
fn default_delay_tone() -> f32 { 3500.0 }

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SlicePersisted {
    pub start: usize,
    pub end: usize,
    pub note: u8,
    pub gain: f32,
    pub pan: f32,
    pub pitch_semitones: f32,
    pub reverse: bool,
    pub muted: bool,
    #[serde(default)]
    pub fade_in_ms: f32,
    #[serde(default)]
    pub fade_out_ms: f32,
    // Per-slice FX
    #[serde(default)]
    pub filter_mode: u8,
    #[serde(default = "default_cutoff")]
    pub filter_cutoff: f32,
    #[serde(default = "default_resonance")]
    pub filter_resonance: f32,
    #[serde(default = "default_bit_depth")]
    pub bit_depth: f32,
    #[serde(default = "default_downsample")]
    pub downsample_factor: u32,
    #[serde(default)]
    pub drive: f32,
    #[serde(default)]
    pub retrigger_rate: u8,
    #[serde(default)]
    pub retrigger_decay: f32,
    #[serde(default)]
    pub choke_group: u8,
    #[serde(default = "default_stretch_factor")]
    pub stretch_factor: f32,
    #[serde(default = "default_stretch_grain_ms")]
    pub stretch_grain_ms: f32,
    #[serde(default)]
    pub delay_rate: u8,
    #[serde(default = "default_delay_feedback")]
    pub delay_feedback: f32,
    #[serde(default = "default_delay_mix")]
    pub delay_mix: f32,
    #[serde(default = "default_delay_tone")]
    pub delay_tone: f32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SlicePlayerPersistedData {
    pub file_path: Option<PathBuf>,
    pub bpm: f64,
    pub sample_rate: u32,
    #[serde(default)]
    pub loop_start: usize,
    #[serde(default)]
    pub loop_end: usize,
    pub slices: Vec<SlicePersisted>,
    pub audio_pcm: Vec<f32>,
}

// ── Parameters ────────────────────────────────────────────────────────────────
#[derive(Params)]
struct SlicePlayerParams {
    #[persist = "editor-state"]
    editor_state: Arc<EguiState>,

    #[id = "gain"]
    pub master_gain: FloatParam,

    #[persist = "persisted_loop"]
    pub persisted_loop: RwLock<Option<SlicePlayerPersistedData>>,

    #[persist = "last_dir"]
    pub last_dir: Arc<Mutex<Option<PathBuf>>>,

    #[persist = "favorites"]
    pub favorites: Arc<Mutex<[Option<PathBuf>; 5]>>,
}

impl Default for SlicePlayerParams {
    fn default() -> Self {
        let global = settings::load_global_settings();
        Self {
            editor_state: EguiState::from_size(900, 500),
            master_gain: FloatParam::new(
                "Master Gain",
                1.0,
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            .with_unit("x")
            .with_step_size(0.01),
            persisted_loop: RwLock::new(None),
            last_dir: Arc::new(Mutex::new(global.last_dir)),
            favorites: Arc::new(Mutex::new(global.favorites)),
        }
    }
}

pub fn sync_global_settings(last_dir: &Mutex<Option<PathBuf>>, favorites: &Mutex<[Option<PathBuf>; 5]>) {
    let last = last_dir.lock().ok().and_then(|g| g.clone());
    let favs = favorites.lock().ok().map(|g| g.clone()).unwrap_or([None, None, None, None, None]);
    let global = settings::GlobalSettings {
        favorites: favs,
        last_dir: last,
        default_export_dir: None,
    };
    settings::save_global_settings(&global);
}

// ── Persistence Helpers ───────────────────────────────────────────────────────
fn update_persisted_from_loop(target: &RwLock<Option<SlicePlayerPersistedData>>, loop_data: Option<&SliceLoop>) {
    let Ok(mut guard) = target.write() else { return; };
    if let Some(sl) = loop_data {
        let slices: Vec<SlicePersisted> = sl.slices.iter().map(|s| SlicePersisted {
            start: s.start, end: s.end, note: s.note, gain: s.gain,
            pan: s.pan, pitch_semitones: s.pitch_semitones,
            reverse: s.reverse, muted: s.muted,
            fade_in_ms: s.fade_in_ms, fade_out_ms: s.fade_out_ms,
            filter_mode: match s.fx.filter_mode {
                slicer::FilterMode::Off => 0,
                slicer::FilterMode::Lowpass => 1,
                slicer::FilterMode::Highpass => 2,
                slicer::FilterMode::Bandpass => 3,
            },
            filter_cutoff: s.fx.filter_cutoff,
            filter_resonance: s.fx.filter_resonance,
            bit_depth: s.fx.bit_depth,
            downsample_factor: s.fx.downsample_factor,
            drive: s.fx.drive,
            retrigger_rate: match s.fx.retrigger_rate {
                slicer::RetriggerRate::Off => 0,
                slicer::RetriggerRate::Eighth => 1,
                slicer::RetriggerRate::Sixteenth => 2,
                slicer::RetriggerRate::ThirtySecond => 3,
                slicer::RetriggerRate::SixtyFourth => 4,
            },
            retrigger_decay: s.fx.retrigger_decay,
            choke_group: s.fx.choke_group,
            stretch_factor: s.fx.stretch_factor,
            stretch_grain_ms: s.fx.stretch_grain_ms,
            delay_rate: match s.fx.delay_rate {
                slicer::DelayRate::Off => 0,
                slicer::DelayRate::Sixteenth => 1,
                slicer::DelayRate::Eighth => 2,
                slicer::DelayRate::DottedEighth => 3,
                slicer::DelayRate::Quarter => 4,
                slicer::DelayRate::Half => 5,
            },
            delay_feedback: s.fx.delay_feedback,
            delay_mix: s.fx.delay_mix,
            delay_tone: s.fx.delay_tone,
        }).collect();

        if let Some(ref mut persisted) = *guard {
            persisted.file_path = sl.file_path.clone();
            persisted.bpm = sl.bpm;
            persisted.sample_rate = sl.sample_rate;
            persisted.loop_start = sl.loop_start;
            persisted.loop_end = sl.loop_end;
            persisted.slices = slices;
            if persisted.audio_pcm.len() != sl.audio.len() {
                persisted.audio_pcm = sl.audio.clone();
            }
        } else {
            *guard = Some(SlicePlayerPersistedData {
                file_path: sl.file_path.clone(),
                bpm: sl.bpm,
                sample_rate: sl.sample_rate,
                loop_start: sl.loop_start,
                loop_end: sl.loop_end,
                slices,
                audio_pcm: sl.audio.clone(),
            });
        }
    } else {
        *guard = None;
    }
}

fn restore_from_persisted(persisted: &RwLock<Option<SlicePlayerPersistedData>>, target: &RwLock<Option<SliceLoop>>) {
    let Ok(guard) = persisted.read() else { return; };
    if let Some(state) = guard.as_ref() {
        let mut sl = None;
        if let Some(ref path) = state.file_path {
            if path.exists() {
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                match ext.as_str() {
                    "wav" => { sl = SliceLoop::load_wav(path).ok(); }
                    "rx2" | "rex" | "rcy" => { sl = SliceLoop::load_rex2(path).ok(); }
                    _ => {}
                }
            }
        }

        if sl.is_none() && !state.audio_pcm.is_empty() {
            let total_frames = state.audio_pcm.len() / 2;
            let mut restored = SliceLoop {
                file_path: state.file_path.clone(),
                audio: state.audio_pcm.clone(),
                channels: 2,
                sample_rate: state.sample_rate,
                total_frames,
                loop_start: 0,
                loop_end: total_frames,
                bpm: state.bpm,
                slices: Vec::new(),
                peak_cache: Vec::new(),
            };
            restored.rebuild_peaks(1024);
            sl = Some(restored);
        }

        if let Some(mut sl) = sl {
            sl.bpm = state.bpm;
            if state.loop_end > 0 {
                sl.loop_start = state.loop_start.min(sl.total_frames);
                sl.loop_end = state.loop_end.min(sl.total_frames);
            }
            if !state.slices.is_empty() {
                sl.slices = state.slices.iter().map(|s| {
                    let mut slice = slicer::Slice::new(s.start, s.end, s.note);
                    slice.gain = s.gain;
                    slice.pan = s.pan;
                    slice.pitch_semitones = s.pitch_semitones;
                    slice.reverse = s.reverse;
                    slice.muted = s.muted;
                    slice.fade_in_ms = s.fade_in_ms;
                    slice.fade_out_ms = s.fade_out_ms;
                    slice.fx.filter_mode = match s.filter_mode {
                        1 => slicer::FilterMode::Lowpass,
                        2 => slicer::FilterMode::Highpass,
                        3 => slicer::FilterMode::Bandpass,
                        _ => slicer::FilterMode::Off,
                    };
                    slice.fx.filter_cutoff = if s.filter_cutoff > 0.0 { s.filter_cutoff } else { 20000.0 };
                    slice.fx.filter_resonance = if s.filter_resonance > 0.0 { s.filter_resonance } else { 0.707 };
                    slice.fx.bit_depth = if s.bit_depth > 0.0 { s.bit_depth } else { 16.0 };
                    slice.fx.downsample_factor = s.downsample_factor.max(1);
                    slice.fx.drive = s.drive;
                    slice.fx.retrigger_rate = match s.retrigger_rate {
                        1 => slicer::RetriggerRate::Eighth,
                        2 => slicer::RetriggerRate::Sixteenth,
                        3 => slicer::RetriggerRate::ThirtySecond,
                        4 => slicer::RetriggerRate::SixtyFourth,
                        _ => slicer::RetriggerRate::Off,
                    };
                    slice.fx.retrigger_decay = s.retrigger_decay;
                    slice.fx.choke_group = s.choke_group;
                    slice.fx.stretch_factor = if s.stretch_factor > 0.0 { s.stretch_factor } else { 1.0 };
                    slice.fx.stretch_grain_ms = if s.stretch_grain_ms > 0.0 { s.stretch_grain_ms } else { 30.0 };
                    slice.fx.delay_rate = match s.delay_rate {
                        1 => slicer::DelayRate::Sixteenth,
                        2 => slicer::DelayRate::Eighth,
                        3 => slicer::DelayRate::DottedEighth,
                        4 => slicer::DelayRate::Quarter,
                        5 => slicer::DelayRate::Half,
                        _ => slicer::DelayRate::Off,
                    };
                    slice.fx.delay_feedback = s.delay_feedback;
                    slice.fx.delay_mix = s.delay_mix;
                    slice.fx.delay_tone = if s.delay_tone > 0.0 { s.delay_tone } else { 3500.0 };
                    slice
                }).collect();
            }
            if let Ok(mut target_guard) = target.write() {
                *target_guard = Some(sl);
            }
        }
    }
}

// ── Plugin struct ─────────────────────────────────────────────────────────────
struct SlicePlayer {
    params: Arc<SlicePlayerParams>,
    /// Audio engine (behind Mutex for audio/GUI sync).
    engine: Arc<Mutex<Engine>>,
    /// Loaded loop data shared between GUI and audio thread.
    loop_data: Arc<RwLock<Option<SliceLoop>>>,
    /// Track DAW transport playback state to stop UI preview when DAW starts.
    was_daw_playing: bool,
    /// Pre-allocated scratch buffer for audio rendering without heap allocation in process().
    scratch_buffer: Vec<f32>,
}

impl Default for SlicePlayer {
    fn default() -> Self {
        let loop_data = Arc::new(RwLock::new(None::<SliceLoop>));
        Self {
            params: Arc::new(SlicePlayerParams::default()),
            engine: Arc::new(Mutex::new(Engine::new())),
            loop_data,
            was_daw_playing: false,
            scratch_buffer: Vec::with_capacity(8192),
        }
    }
}

// ── nih-plug Plugin impl ──────────────────────────────────────────────────────
impl Plugin for SlicePlayer {
    const NAME: &'static str = "SlicePlayer";
    const VENDOR: &'static str = "Aura";
    const URL: &'static str = "https://github.com/aura";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &[],
            aux_output_ports: &[],
            names: PortNames::const_default(),
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let loop_data = self.loop_data.clone();
        let engine    = self.engine.clone();
        let params    = self.params.clone();

        // Restore loop data from DAW project state if missing
        if let Ok(guard) = loop_data.read() {
            if guard.is_none() {
                drop(guard);
                restore_from_persisted(&params.persisted_loop, &loop_data);
            }
        }

        create_egui_editor(
            self.params.editor_state.clone(),
            // Initial state for the editor.
            EditorState::new(loop_data.clone(), engine.clone(), params.last_dir.clone(), params.favorites.clone()),
            // Build/init callback — called once when the window opens.
            |ctx, _state| {
                ctx.set_style(egui::Style {
                    visuals: egui::Visuals::dark(),
                    ..Default::default()
                });
            },
            // Update callback — called every frame.
            move |ctx, _setter, state| {
                // Sync master gain from engine.
                if let Ok(mut eng) = engine.lock() {
                    eng.master_gain = params.master_gain.value();
                }

                // Sync loop data to persisted state for DAW saves
                if let Ok(guard) = loop_data.read() {
                    update_persisted_from_loop(&params.persisted_loop, guard.as_ref());
                }

                ResizableWindow::new("slice_player_window")
                    .min_size([700.0, 400.0])
                    .show(ctx, &params.editor_state, |ui| {
                        editor::draw(ui, state);
                    });
            },
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        restore_from_persisted(&self.params.persisted_loop, &self.loop_data);
        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Stop internal UI loop preview if DAW transport starts playing
        let is_playing = context.transport().playing;
        if is_playing && !self.was_daw_playing {
            if let Ok(mut eng) = self.engine.lock() {
                eng.stop_preview();
                eng.stop_audition();
            }
        }
        self.was_daw_playing = is_playing;

        // Handle incoming MIDI events.
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { note, velocity, voice_id, .. } => {
                    if let Ok(guard) = self.loop_data.read() {
                        if let Some(sl) = guard.as_ref() {
                            if let Ok(mut eng) = self.engine.lock() {
                                eng.note_on(sl, note, velocity, voice_id.unwrap_or(-1));
                            }
                        }
                    }
                }
                NoteEvent::NoteOff { voice_id, .. } => {
                    if let Ok(mut eng) = self.engine.lock() {
                        eng.note_off(voice_id.unwrap_or(-1));
                    }
                }
                _ => {}
            }
        }

        // Render audio.
        if let Ok(guard) = self.loop_data.read() {
            if let Some(sl) = guard.as_ref() {
                let frames = buffer.samples();
                let channels = buffer.channels();
                if channels > 0 {
                    let block = buffer.as_slice();
                    let needed_len = frames * 2;
                    if self.scratch_buffer.len() < needed_len {
                        self.scratch_buffer.resize(needed_len, 0.0);
                    }
                    self.scratch_buffer[..needed_len].fill(0.0);

                    if let Ok(mut eng) = self.engine.lock() {
                        eng.process(&mut self.scratch_buffer[..needed_len], frames, sl);
                    }

                    if channels == 1 {
                        for f in 0..frames {
                            block[0][f] = (self.scratch_buffer[f * 2] + self.scratch_buffer[f * 2 + 1]) * 0.5;
                        }
                    } else {
                        #[allow(clippy::needless_range_loop)]
                        for f in 0..frames {
                            block[0][f] = self.scratch_buffer[f * 2];
                            block[1][f] = self.scratch_buffer[f * 2 + 1];
                        }
                        for ch in block.iter_mut().take(channels).skip(2) {
                            for sample in ch.iter_mut().take(frames) {
                                *sample = 0.0;
                            }
                        }
                    }
                }
            }
        }

        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for SlicePlayer {
    const CLAP_ID: &'static str = "com.aura.slice_player";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("REX2/WAV Sample Slice Player with editor");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Sampler,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for SlicePlayer {
    const VST3_CLASS_ID: [u8; 16] = *b"AuraSlicePlayer!";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Sampler];
}

nih_export_clap!(SlicePlayer);
nih_export_vst3!(SlicePlayer);
