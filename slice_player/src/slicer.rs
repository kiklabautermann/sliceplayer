//! Slice data model and all three slicing methods:
//! 1. Manual (add/move/remove via editor)
//! 2. Grid (1/4, 1/8, 1/16, 1/32 at a given BPM)
//! 3. Transient detection (VelociLoops SuperFlux)

use std::path::{Path, PathBuf};
use crate::velocloops_ffi::{Rex2File, superflux_detect_slices, superflux_default_options};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

// ── Grid division ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridDivision {
    Quarter,   // 1/4
    Eighth,    // 1/8
    Sixteenth, // 1/16
    ThirtySecond, // 1/32
}

impl GridDivision {
    pub fn beats_per_slice(self) -> f64 {
        match self {
            Self::Quarter      => 1.0,
            Self::Eighth       => 0.5,
            Self::Sixteenth    => 0.25,
            Self::ThirtySecond => 0.125,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Quarter      => "1/4",
            Self::Eighth       => "1/8",
            Self::Sixteenth    => "1/16",
            Self::ThirtySecond => "1/32",
        }
    }
}

// ── Transient detection settings ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TransientSettings {
    /// Peak-picking threshold over local average (default 1.1, range ~0.8–3.0).
    pub threshold: f32,
    /// Minimum gap between slices in milliseconds (prevents double-hits).
    pub min_gap_ms: f32,
    /// Pre-roll: move each marker N ms before the detected peak.
    pub pre_roll_ms: f32,
}

impl Default for TransientSettings {
    fn default() -> Self {
        Self { threshold: 0.30, min_gap_ms: 30.0, pre_roll_ms: 0.0 }
    }
}

// ── Per-slice DSP & FX settings ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    Off,
    Lowpass,
    Highpass,
    Bandpass,
}

impl FilterMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Lowpass => "Lowpass",
            Self::Highpass => "Highpass",
            Self::Bandpass => "Bandpass",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetriggerRate {
    Off,
    Eighth,       // 1/8
    Sixteenth,    // 1/16
    ThirtySecond, // 1/32
    SixtyFourth,  // 1/64
}

impl RetriggerRate {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Eighth => "1/8",
            Self::Sixteenth => "1/16",
            Self::ThirtySecond => "1/32",
            Self::SixtyFourth => "1/64",
        }
    }

    pub fn division_factor(self) -> usize {
        match self {
            Self::Off => 0,
            Self::Eighth => 8,
            Self::Sixteenth => 16,
            Self::ThirtySecond => 32,
            Self::SixtyFourth => 64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelayRate {
    Off,
    Ms,              // Free Milliseconds mode (1.0ms - 100.0ms)
    SixtyFourth,     // 1/64
    ThirtySecond,    // 1/32
    Sixteenth,       // 1/16
    DottedSixteenth, // 1/16 Dotted (0.375 beats)
    Eighth,          // 1/8
    DottedEighth,    // 3/16 Dotted (0.75 beats)
    Quarter,         // 1/4
    Half,            // 1/2
}

impl DelayRate {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Ms => "Free ms",
            Self::SixtyFourth => "1/64",
            Self::ThirtySecond => "1/32",
            Self::Sixteenth => "1/16",
            Self::DottedSixteenth => "1/16 Dotted",
            Self::Eighth => "1/8",
            Self::DottedEighth => "3/16 Dotted",
            Self::Quarter => "1/4",
            Self::Half => "1/2",
        }
    }

    pub fn beats(self) -> f32 {
        match self {
            Self::Off | Self::Ms => 0.0,
            Self::SixtyFourth => 0.0625,
            Self::ThirtySecond => 0.125,
            Self::Sixteenth => 0.25,
            Self::DottedSixteenth => 0.375,
            Self::Eighth => 0.5,
            Self::DottedEighth => 0.75,
            Self::Quarter => 1.0,
            Self::Half => 2.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ShuffleStyle {
    AmenRoller,
    SyncopatedFunk,
    GhostNotesOnly,
    WildChopper,
}

impl ShuffleStyle {
    pub fn label(self) -> &'static str {
        match self {
            Self::AmenRoller => "Amen Roller",
            Self::SyncopatedFunk => "Syncopated Funk",
            Self::GhostNotesOnly => "Ghost Notes Only",
            Self::WildChopper => "Wild Chopper",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SliceFx {
    pub filter_mode: FilterMode,
    /// Cutoff frequency in Hz (20.0 Hz - 20000.0 Hz).
    pub filter_cutoff: f32,
    /// DJM combo filter position (-1.0 = full LP cut, 0.0 = Off/Neutral, +1.0 = full HP cut).
    pub filter_djm: f32,
    /// Filter resonance / Q (0.5 - 10.0).
    pub filter_resonance: f32,

    /// Bit reduction depth (1.0 - 16.0 bits, 16.0 = off).
    pub bit_depth: f32,
    /// Sample rate downsample factor (1 - 32x, 1 = off).
    pub downsample_factor: u32,
    /// Drive / Soft clipping saturation (0.0 - 1.0).
    pub drive: f32,

    /// Retrigger rate.
    pub retrigger_rate: RetriggerRate,
    /// Retrigger volume decay per roll hit (0.0 = no decay, 1.0 = quick decay).
    pub retrigger_decay: f32,

    /// Choke Group (0 = None, 1..=8).
    pub choke_group: u8,

    /// Akai-style granular vintage timestretch ratio (0.5x - 2.0x, 1.0 = Off).
    pub stretch_factor: f32,
    /// Granular window length in ms (10.0ms - 100.0ms, default 30.0ms).
    pub stretch_grain_ms: f32,

    /// DJM500 / Oldschool Jungle Dub Echo rate.
    pub delay_rate: DelayRate,
    /// Free delay time in milliseconds (1.0ms - 100.0ms).
    pub delay_ms: f32,
    /// Delay feedback (0.0 - 0.90).
    pub delay_feedback: f32,
    /// Delay wet mix (0.0 - 1.0).
    pub delay_mix: f32,
    /// Delay feedback lowpass filter tone (200.0 Hz - 12000.0 Hz).
    pub delay_tone: f32,
}

impl SliceFx {
    pub fn effective_filter(&self) -> (FilterMode, f32) {
        if self.filter_djm < -0.01 {
            let norm = 1.0 + self.filter_djm.clamp(-1.0, 0.0);
            let cutoff = 20.0f32 * (20000.0f32 / 20.0f32).powf(norm);
            (FilterMode::Lowpass, cutoff)
        } else if self.filter_djm > 0.01 {
            let norm = self.filter_djm.clamp(0.0, 1.0);
            let cutoff = 20.0f32 * (15000.0f32 / 20.0f32).powf(norm);
            (FilterMode::Highpass, cutoff)
        } else if self.filter_mode != FilterMode::Off {
            (self.filter_mode, self.filter_cutoff)
        } else {
            (FilterMode::Off, 20000.0)
        }
    }
}

impl Default for SliceFx {
    fn default() -> Self {
        Self {
            filter_mode: FilterMode::Off,
            filter_cutoff: 20000.0,
            filter_djm: 0.0,
            filter_resonance: 0.707,
            bit_depth: 16.0,
            downsample_factor: 1,
            drive: 0.0,
            retrigger_rate: RetriggerRate::Off,
            retrigger_decay: 0.0,
            choke_group: 0,
            stretch_factor: 1.0,
            stretch_grain_ms: 30.0,
            delay_rate: DelayRate::Off,
            delay_ms: 20.0,
            delay_feedback: 0.45,
            delay_mix: 0.35,
            delay_tone: 3500.0,
        }
    }
}

// ── Per-slice data ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    /// Start sample frame (in the parent loop's sample array).
    pub start: usize,
    /// End sample frame (exclusive). Equal to the next slice's start, or total_frames.
    pub end: usize,
    /// MIDI note this slice is triggered by (C3=48 default for slice 0, +1 per slice).
    pub note: u8,
    pub gain: f32,
    pub pan: f32,
    /// Pitch shift in semitones (simple resample).
    pub pitch_semitones: f32,
    pub reverse: bool,
    pub muted: bool,
    /// Volume Fade-In duration in milliseconds.
    pub fade_in_ms: f32,
    /// Volume Fade-Out duration in milliseconds.
    pub fade_out_ms: f32,
    /// Per-slice DSP / FX settings.
    pub fx: SliceFx,
}

impl Slice {
    pub fn new(start: usize, end: usize, note: u8) -> Self {
        Self {
            start, end, note,
            gain: 1.0, pan: 0.0, pitch_semitones: 0.0,
            reverse: false, muted: false,
            fade_in_ms: 0.0, fade_out_ms: 0.0,
            fx: SliceFx::default(),
        }
    }
    pub fn frame_count(&self) -> usize { self.end.saturating_sub(self.start) }

    pub fn fade_in_frames(&self, sample_rate: u32) -> usize {
        let max_frames = self.frame_count() / 2;
        ((self.fade_in_ms * sample_rate as f32 / 1000.0) as usize).min(max_frames)
    }

    pub fn fade_out_frames(&self, sample_rate: u32) -> usize {
        let max_frames = self.frame_count() / 2;
        ((self.fade_out_ms * sample_rate as f32 / 1000.0) as usize).min(max_frames)
    }

    pub fn reset_fx(&mut self) {
        self.fx = SliceFx::default();
    }

    pub fn copy_fx_from(&mut self, source: &Slice) {
        self.fx = source.fx.clone();
    }
}

// ── Main loop struct ──────────────────────────────────────────────────────────

pub struct SliceLoop {
    pub file_path: Option<PathBuf>,
    /// Interleaved stereo float audio (L, R, L, R, …). Mono files are duplicated.
    pub audio: Vec<f32>,
    #[allow(dead_code)]
    pub channels: usize,
    pub sample_rate: u32,
    pub total_frames: usize,
    /// Loop Start frame offset.
    pub loop_start: usize,
    /// Loop End frame offset.
    pub loop_end: usize,
    /// BPM from file metadata (REX2) or user-set.
    pub bpm: f64,
    pub slices: Vec<Slice>,
    /// Waveform peak cache for the GUI (one peak per pixel bucket).
    pub peak_cache: Vec<(f32, f32)>,
}

impl SliceLoop {
    // ── Universal Audio Import (WAV, AIFF, FLAC, MP3, M4A, AAC, OPUS, OGG) ───

    pub fn load_audio(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("Open file error: {e}"))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions { enable_gapless: true, ..Default::default() };
        let metadata_opts = MetadataOptions::default();
        let decoder_opts = DecoderOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| format!("Audio format decode error: {e}"))?;

        let mut format = probed.format;
        let track = format.default_track()
            .ok_or_else(|| "No default audio track found in file".to_string())?;

        let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_opts)
            .map_err(|e| format!("Codec decoder creation error: {e}"))?;

        let mut audio: Vec<f32> = Vec::new();

        while let Ok(packet) = format.next_packet() {
            match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    let spec = *audio_buf.spec();
                    let num_channels = spec.channels.count();
                    let num_frames = audio_buf.frames();

                    let mut sample_buf = SampleBuffer::<f32>::new(audio_buf.capacity() as u64, spec);
                    sample_buf.copy_interleaved_ref(audio_buf);

                    let samples = sample_buf.samples();

                    if num_channels == 1 {
                        for &s in samples {
                            audio.push(s);
                            audio.push(s);
                        }
                    } else if num_channels >= 2 {
                        for frame_idx in 0..num_frames {
                            audio.push(samples[frame_idx * num_channels]);
                            audio.push(samples[frame_idx * num_channels + 1]);
                        }
                    }
                }
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                Err(_) => break,
            }
        }

        let total_frames = audio.len() / 2;
        if total_frames == 0 {
            return Err("No audio frames decoded".into());
        }

        let bpm = detect_wav_bpm(path, sample_rate, total_frames);

        let mut s = Self {
            file_path: Some(path.to_path_buf()),
            audio, channels: 2, sample_rate, total_frames,
            loop_start: 0, loop_end: total_frames,
            bpm, slices: Vec::new(), peak_cache: Vec::new(),
        };
        s.rebuild_peaks(1024);
        Ok(s)
    }

    pub fn load_wav(path: &Path) -> Result<Self, String> {
        Self::load_audio(path)
    }

    // ── REX2 import ───────────────────────────────────────────────────────────

    pub fn load_rex2(path: &Path) -> Result<Self, String> {
        let rex = Rex2File::open(path)?;
        let info = rex.info.clone();
        let sample_rate = info.sample_rate as u32;
        let bpm = rex.bpm();

        // Decode all slices and concatenate into one audio buffer.
        let mut audio: Vec<f32> = Vec::new();
        let mut slices: Vec<Slice> = Vec::new();
        let base_note: u8 = 48; // C3

        for i in 0..info.slice_count {
            let start_frame = audio.len() / 2;
            let stereo = rex.decode_slice_stereo(i)?;
            let frame_count = stereo.len() / 2;
            audio.extend_from_slice(&stereo);
            let end_frame = start_frame + frame_count;
            let note = (base_note as i32 + i).min(127) as u8;
            slices.push(Slice::new(start_frame, end_frame, note));
        }

        let raw_frames = audio.len() / 2;
        let total_frames = if info.total_frames > 0 {
            (info.total_frames as usize).min(raw_frames)
        } else if info.ppq_length > 0 && bpm > 0.0 {
            let bars = info.ppq_length as f64 / 6144.0;
            let duration_secs = bars * 4.0 * 60.0 / bpm;
            ((duration_secs * sample_rate as f64) as usize).min(raw_frames)
        } else {
            raw_frames
        };

        // Fix slice end points so they don't exceed total_frames.
        for slice in &mut slices {
            slice.start = slice.start.min(total_frames);
            slice.end = slice.end.min(total_frames);
        }
        if let Some(last) = slices.last_mut() {
            last.end = total_frames;
        }

        let mut s = Self {
            file_path: Some(path.to_path_buf()),
            audio, channels: 2, sample_rate, total_frames,
            loop_start: 0, loop_end: total_frames,
            bpm, slices, peak_cache: Vec::new(),
        };
        s.rebuild_peaks(1024);
        Ok(s)
    }

    // ── Slice methods ─────────────────────────────────────────────────────────

    pub fn loop_frames(&self) -> usize {
        self.loop_end.saturating_sub(self.loop_start)
    }

    pub fn calculate_bars(&self) -> f64 {
        let duration_secs = self.loop_frames() as f64 / self.sample_rate as f64;
        if duration_secs > 0.0 && self.bpm > 0.0 {
            (duration_secs / 60.0 * self.bpm) / 4.0
        } else {
            1.0
        }
    }

    pub fn update_bpm_from_bars(&mut self, bars: f64) {
        let duration_secs = self.loop_frames() as f64 / self.sample_rate as f64;
        if duration_secs > 0.0 && bars > 0.0 {
            self.bpm = (bars * 4.0 / duration_secs) * 60.0;
        }
    }

    /// Safely change the MIDI note for a slice, swapping with any slice that currently holds `new_note`.
    pub fn set_slice_note(&mut self, slice_idx: usize, new_note: u8) {
        if slice_idx >= self.slices.len() { return; }
        let old_note = self.slices[slice_idx].note;
        if old_note == new_note { return; }

        for (i, slice) in self.slices.iter_mut().enumerate() {
            if i != slice_idx && slice.note == new_note {
                slice.note = old_note;
            }
        }
        self.slices[slice_idx].note = new_note;
    }

    /// Replace slices with an even grid within [loop_start..loop_end].
    pub fn apply_grid(&mut self, division: GridDivision, bpm: f64) {
        self.bpm = bpm;
        let beats_per_slice = division.beats_per_slice();
        let samples_per_beat = (self.sample_rate as f64 * 60.0 / bpm) as usize;
        let samples_per_slice = ((samples_per_beat as f64 * beats_per_slice) as usize).max(1);

        self.slices.clear();
        let mut start = 0usize;
        let mut note: u8 = 48;
        while start < self.total_frames {
            let end = (start + samples_per_slice).min(self.total_frames);
            self.slices.push(Slice::new(start, end, note));
            note = note.saturating_add(1).min(127);
            start = end;
        }
        self.fix_slice_ends();
    }

    /// Detect transients using VelociLoops SuperFlux and set slices.
    pub fn detect_transients(&mut self, settings: &TransientSettings, bpm: f64) -> Result<(), String> {
        self.bpm = bpm;

        // Build options from our settings.
        let mut opts = superflux_default_options();
        opts.threshold = settings.threshold;
        // combine_ms = min gap between slices.
        opts.combine_ms = settings.min_gap_ms;
        // min_slice_frames from min_gap_ms.
        opts.min_slice_frames = (settings.min_gap_ms * self.sample_rate as f32 / 1000.0) as i32;

        // Extract L/R deinterleaved for VelociLoops.
        let (left, right) = self.deinterleave();

        let mut positions = superflux_detect_slices(&left, Some(&right), self.sample_rate, bpm, &opts)?;

        // Apply pre-roll (shift markers earlier).
        if settings.pre_roll_ms > 0.0 {
            let pre_roll_samples = (settings.pre_roll_ms * self.sample_rate as f32 / 1000.0) as usize;
            for p in positions.iter_mut() {
                *p = (*p as usize).saturating_sub(pre_roll_samples) as i32;
            }
        }

        // Filter positions strictly within [loop_start..loop_end]
        let mut loop_positions: Vec<usize> = positions.iter()
            .filter(|&&p| p > 0)
            .map(|&p| p as usize)
            .filter(|&p| p > self.loop_start && p < self.loop_end)
            .collect();

        // Always start at loop_start.
        loop_positions.insert(0, self.loop_start);

        // Build slices.
        self.slices.clear();
        for (i, &start) in loop_positions.iter().enumerate() {
            let end = loop_positions.get(i + 1).copied().unwrap_or(self.loop_end).min(self.total_frames);
            let note = (48i32 + i as i32).min(127) as u8;
            self.slices.push(Slice::new(start, end, note));
        }
        self.fix_slice_ends();
        Ok(())
    }

    /// Generative Jungle / Drum & Bass breakbeat shuffler.
    pub fn apply_jungle_break_shuffle(&mut self, style: ShuffleStyle, intensity: f32, lock_main_beats: bool) {
        // If there is only 1 slice, auto-slice into a 16th grid first!
        if self.slices.len() <= 1 {
            self.apply_grid(GridDivision::Sixteenth, self.bpm);
        }
        let num_slices = self.slices.len();
        if num_slices == 0 { return; }

        let intensity_clamped = intensity.clamp(0.05, 1.0);

        // Simple pseudo-random helper for deterministic/fun variations
        let mut seed = (num_slices * 31 + (intensity_clamped * 100.0) as usize) as u32;
        let mut rand_f32 = || -> f32 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 9) as f32 / 8388608.0
        };

        match style {
            ShuffleStyle::GhostNotesOnly => {
                for (idx, slice) in self.slices.iter_mut().enumerate() {
                    let is_main_beat = idx % 4 == 0; // 1/4 beats
                    if lock_main_beats && is_main_beat { continue; }

                    if !is_main_beat && rand_f32() < intensity_clamped {
                        // Shape into snappy ghost note: subtle pitch shift & short fade out (preserve gain)
                        slice.pitch_semitones = (rand_f32() * 3.0).round(); // +0..+3 semitones
                        slice.fade_out_ms = (15.0 + rand_f32() * 25.0).clamp(5.0, 50.0);
                        slice.reverse = false;
                    }
                }
            }
            ShuffleStyle::AmenRoller => {
                for (idx, slice) in self.slices.iter_mut().enumerate() {
                    let is_main_beat = idx % 4 == 0;
                    if lock_main_beats && is_main_beat { continue; }

                    let roll_prob = rand_f32();
                    if roll_prob < intensity_clamped * 0.40 {
                        // Retrigger roll on fill slices
                        slice.fx.retrigger_rate = if rand_f32() > 0.5 { RetriggerRate::ThirtySecond } else { RetriggerRate::Sixteenth };
                        slice.fx.retrigger_decay = 0.6 + rand_f32() * 0.35;
                        slice.pitch_semitones = (rand_f32() * 5.0).round(); // Pitch rise
                    } else if roll_prob < intensity_clamped * 0.70 {
                        // Filter accent (preserve gain)
                        slice.fx.filter_djm = 0.15 + rand_f32() * 0.30; // Subtle HP filter cut
                    }
                }

                // Swap offbeat slices for classic Amen shuffle fill
                if num_slices >= 8 {
                    let swap_count = ((num_slices as f32 * 0.25 * intensity_clamped) as usize).max(1);
                    for _ in 0..swap_count {
                        let i1 = (rand_f32() * num_slices as f32) as usize % num_slices;
                        let i2 = (rand_f32() * num_slices as f32) as usize % num_slices;
                        if lock_main_beats && (i1 % 4 == 0 || i2 % 4 == 0) { continue; }
                        if i1 != i2 {
                            let note1 = self.slices[i1].note;
                            self.slices[i1].note = self.slices[i2].note;
                            self.slices[i2].note = note1;
                        }
                    }
                }
            }
            ShuffleStyle::SyncopatedFunk => {
                for (idx, slice) in self.slices.iter_mut().enumerate() {
                    let is_main_beat = idx % 4 == 0;
                    if lock_main_beats && is_main_beat { continue; }

                    let p = rand_f32();
                    if p < intensity_clamped * 0.50 {
                        // Funky swing pitch offset & drive accent (preserve gain)
                        slice.pitch_semitones = if rand_f32() > 0.6 { 2.0 } else { 0.0 };
                        slice.fx.drive = (rand_f32() * 0.25).clamp(0.0, 0.4);
                    }
                }
            }
            ShuffleStyle::WildChopper => {
                for (idx, slice) in self.slices.iter_mut().enumerate() {
                    let is_main_beat = idx % 4 == 0;
                    if lock_main_beats && is_main_beat { continue; }

                    if rand_f32() < intensity_clamped {
                        let r = rand_f32();
                        if r < 0.25 {
                            slice.reverse = !slice.reverse;
                        } else if r < 0.50 {
                            slice.fx.retrigger_rate = RetriggerRate::ThirtySecond;
                            slice.fx.retrigger_decay = 0.5;
                        } else if r < 0.75 {
                            slice.pitch_semitones = (rand_f32() * 12.0 - 6.0).round(); // -6..+6 st
                        } else {
                            slice.fx.bit_depth = 8.0;
                            slice.fx.downsample_factor = 2;
                        }
                    }
                }
            }
        }
        self.rebuild_peaks(1024);
    }

    // ── Manual slice editing ──────────────────────────────────────────────────

    /// Insert a new slice boundary at `frame`. Splits the existing slice that
    /// contains this frame. Does nothing if frame is at an existing boundary.
    pub fn insert_slice_at(&mut self, frame: usize) {
        if frame == 0 || frame >= self.total_frames { return; }

        // Find the slice that contains `frame`.
        let Some(idx) = self.slices.iter().position(|s| s.start < frame && frame < s.end) else { return };

        let existing_end = self.slices[idx].end;
        let existing_note = self.slices[idx].note;

        // Shorten existing slice.
        self.slices[idx].end = frame;

        // Insert new slice after it.
        let new_note = existing_note.saturating_add(1).min(127);
        self.slices.insert(idx + 1, Slice::new(frame, existing_end, new_note));

        // Re-assign notes for all following slices.
        self.reassign_notes_from(idx + 1);
    }

    /// Move slice marker at `slice_idx` (its start boundary) to `new_frame`.
    pub fn move_slice_start(&mut self, slice_idx: usize, new_frame: usize) {
        if slice_idx == 0 { return; } // First slice always starts at 0.
        let total = self.total_frames;
        let prev_start = self.slices[slice_idx - 1].start;
        let next_end = self.slices.get(slice_idx + 1).map(|s| s.start).unwrap_or(total);

        let clamped = new_frame.clamp(prev_start + 1, next_end.saturating_sub(1));
        self.slices[slice_idx - 1].end = clamped;
        self.slices[slice_idx].start = clamped;
    }

    /// Remove the slice at `index`, merging it with the previous one.
    pub fn remove_slice(&mut self, index: usize) {
        if self.slices.len() <= 1 || index >= self.slices.len() { return; }
        let removed_end = self.slices[index].end;
        self.slices.remove(index);
        if index > 0 && index - 1 < self.slices.len() {
            self.slices[index - 1].end = removed_end;
        } else if !self.slices.is_empty() {
            self.slices[0].start = 0;
        }
        self.fix_slice_ends();
        let reassign_pos = index.min(self.slices.len().saturating_sub(1));
        self.reassign_notes_from(reassign_pos);
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn fix_slice_ends(&mut self) {
        let total = self.total_frames;
        let n = self.slices.len();
        for i in 0..n {
            let next_start = self.slices.get(i + 1).map(|s| s.start).unwrap_or(total);
            self.slices[i].end = next_start;
        }
        if let Some(last) = self.slices.last_mut() {
            last.end = total;
        }
    }

    fn reassign_notes_from(&mut self, from: usize) {
        if from >= self.slices.len() { return; }
        let base = if from > 0 {
            self.slices.get(from - 1).map(|s| s.note.saturating_add(1)).unwrap_or(48u8)
        } else {
            48u8
        };
        for (i, s) in self.slices[from..].iter_mut().enumerate() {
            s.note = (base as i32 + i as i32).min(127) as u8;
        }
    }

    fn deinterleave(&self) -> (Vec<f32>, Vec<f32>) {
        let n = (self.audio.len() / 2).min(self.total_frames);
        let mut left  = Vec::with_capacity(n);
        let mut right = Vec::with_capacity(n);
        for i in 0..n {
            left.push(self.audio[i * 2]);
            right.push(self.audio[i * 2 + 1]);
        }
        (left, right)
    }

    /// Rebuild the peak cache used by the waveform view.
    /// `buckets` is the number of horizontal pixels available.
    pub fn rebuild_peaks(&mut self, buckets: usize) {
        if self.total_frames == 0 { self.peak_cache.clear(); return; }
        let frames_per_bucket = (self.total_frames as f64 / buckets as f64).max(1.0);
        self.peak_cache = (0..buckets).map(|b| {
            let start = (b as f64 * frames_per_bucket) as usize;
            let end   = ((b + 1) as f64 * frames_per_bucket) as usize;
            let end   = end.min(self.total_frames);
            let mut peak_pos: f32 = 0.0;
            let mut peak_neg: f32 = 0.0;
            for f in start..end {
                let l = self.audio[f * 2];
                let r = self.audio[f * 2 + 1];
                let s = (l + r) * 0.5;
                if s > peak_pos { peak_pos = s; }
                if s < peak_neg { peak_neg = s; }
            }
            (peak_neg, peak_pos)
        }).collect();
    }

    /// Return the sample at position `frame` as a mono float.
    #[allow(dead_code)]
    pub fn sample_mono(&self, frame: usize) -> f32 {
        let idx = frame * 2;
        if idx + 1 < self.audio.len() {
            (self.audio[idx] + self.audio[idx + 1]) * 0.5
        } else {
            0.0
        }
    }

    pub fn copy_slice_fx_to_all(&mut self, src_idx: usize) {
        if let Some(src_fx) = self.slices.get(src_idx).map(|s| s.fx.clone()) {
            for slice in self.slices.iter_mut() {
                slice.fx = src_fx.clone();
            }
        }
    }

    pub fn reset_all_slice_fx(&mut self) {
        for slice in self.slices.iter_mut() {
            slice.reset_fx();
        }
    }

    /// Find the nearest zero-crossing sample frame within a given search window (e.g. ±500 samples ~10ms).
    pub fn snap_to_zero_crossing(&self, frame: usize, max_search_samples: usize) -> usize {
        if self.audio.is_empty() || frame >= self.total_frames {
            return frame;
        }

        let num_channels = self.channels.max(1);
        let start = frame.saturating_sub(max_search_samples);
        let end = (frame + max_search_samples).min(self.total_frames);

        let mut best_frame = frame;
        let mut min_abs_val = f32::MAX;
        let mut min_dist = usize::MAX;

        for f in start..end {
            let idx = f * num_channels;
            if idx < self.audio.len() {
                let sample_l = self.audio[idx];
                let sample_r = if num_channels > 1 && idx + 1 < self.audio.len() { self.audio[idx + 1] } else { sample_l };
                let abs_val = (sample_l.abs() + sample_r.abs()) * 0.5;

                // Check if sign flips between this frame and previous frame (true zero crossing)
                let is_true_crossing = if f > 0 && (f - 1) * num_channels < self.audio.len() {
                    let prev_l = self.audio[(f - 1) * num_channels];
                    (sample_l >= 0.0 && prev_l < 0.0) || (sample_l <= 0.0 && prev_l > 0.0)
                } else {
                    false
                };

                let dist = (f as isize - frame as isize).unsigned_abs();

                if is_true_crossing {
                    if dist < min_dist {
                        min_dist = dist;
                        best_frame = f;
                    }
                } else if min_dist == usize::MAX && abs_val < min_abs_val {
                    min_abs_val = abs_val;
                    best_frame = f;
                }
            }
        }

        best_frame
    }

    /// Find the next true zero-crossing frame to the left (earlier in time) from start_frame.
    pub fn find_zero_crossing_left(&self, start_frame: usize) -> usize {
        if self.audio.is_empty() || start_frame <= 1 { return 0; }
        let num_channels = self.channels.max(1);
        let min_f = start_frame.saturating_sub(10000).max(1);

        for f in (min_f..start_frame).rev() {
            let idx = f * num_channels;
            let prev_idx = (f - 1) * num_channels;
            if idx < self.audio.len() && prev_idx < self.audio.len() {
                let sample_l = self.audio[idx];
                let prev_l = self.audio[prev_idx];
                if (sample_l >= 0.0 && prev_l < 0.0) || (sample_l <= 0.0 && prev_l > 0.0) {
                    return f;
                }
            }
        }
        start_frame.saturating_sub(1)
    }

    /// Find the next true zero-crossing frame to the right (later in time) from start_frame.
    pub fn find_zero_crossing_right(&self, start_frame: usize) -> usize {
        if self.audio.is_empty() || start_frame >= self.total_frames.saturating_sub(1) {
            return self.total_frames;
        }
        let num_channels = self.channels.max(1);
        let max_f = (start_frame + 10000).min(self.total_frames);

        for f in (start_frame + 1)..max_f {
            let idx = f * num_channels;
            let prev_idx = (f - 1) * num_channels;
            if idx < self.audio.len() && prev_idx < self.audio.len() {
                let sample_l = self.audio[idx];
                let prev_l = self.audio[prev_idx];
                if (sample_l >= 0.0 && prev_l < 0.0) || (sample_l <= 0.0 && prev_l > 0.0) {
                    return f;
                }
            }
        }
        (start_frame + 1).min(self.total_frames)
    }
}

fn detect_wav_bpm(path: &Path, _sample_rate: u32, _total_frames: usize) -> f64 {
    // 1. Try reading RIFF 'acid' chunk metadata from raw WAV file bytes
    if let Ok(data) = std::fs::read(path) {
        if let Some(pos) = data.windows(4).position(|w| w == b"acid") {
            if pos + 24 <= data.len() {
                let tempo_bytes = &data[pos + 20..pos + 24];
                let tempo_f32 = f32::from_le_bytes(tempo_bytes.try_into().unwrap_or([0; 4]));
                if (40.0..=300.0).contains(&tempo_f32) {
                    return tempo_f32 as f64;
                }
            }
        }
    }

    // 2. Try extracting BPM from filename (e.g. "loop_128bpm.wav", "174_dnb.wav")
    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
        let name_lower = name.to_lowercase();
        for token in name_lower.split(&['_', '-', ' ', '.'][..]) {
            let num_str = token.trim_matches(|c: char| !c.is_numeric() && c != '.');
            if let Ok(val) = num_str.parse::<f64>() {
                if (50.0..=240.0).contains(&val) && (token.contains("bpm") || name_lower.contains("bpm")) {
                    return val;
                }
            }
        }
    }

    120.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jungle_break_shuffler() {
        let sample_rate: u32 = 44100;
        let total_frames = sample_rate as usize * 2;
        let audio = vec![0.0f32; total_frames * 2];
        let mut sl = SliceLoop {
            file_path: None,
            audio,
            channels: 2,
            sample_rate,
            total_frames,
            loop_start: 0,
            loop_end: total_frames,
            bpm: 174.0,
            slices: Vec::new(),
            peak_cache: Vec::new(),
        };

        sl.apply_grid(GridDivision::Sixteenth, 174.0);
        let n = sl.slices.len();
        assert!(n > 0);

        // Verify that gain of all slices remains untouched (1.0) across all shuffle modes
        for slice in &sl.slices {
            assert_eq!(slice.gain, 1.0);
        }

        sl.apply_jungle_break_shuffle(ShuffleStyle::AmenRoller, 0.70, true);
        assert_eq!(sl.slices.len(), n);
        for slice in &sl.slices { assert_eq!(slice.gain, 1.0); }

        sl.apply_jungle_break_shuffle(ShuffleStyle::GhostNotesOnly, 0.50, false);
        assert_eq!(sl.slices.len(), n);
        for slice in &sl.slices { assert_eq!(slice.gain, 1.0); }

        sl.apply_jungle_break_shuffle(ShuffleStyle::SyncopatedFunk, 0.80, true);
        assert_eq!(sl.slices.len(), n);
        for slice in &sl.slices { assert_eq!(slice.gain, 1.0); }

        sl.apply_jungle_break_shuffle(ShuffleStyle::WildChopper, 1.00, false);
        assert_eq!(sl.slices.len(), n);
        for slice in &sl.slices { assert_eq!(slice.gain, 1.0); }
    }
}
