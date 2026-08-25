//! Polyphonic audio engine with per-slice DSP / FX processing.
//! One Voice per slice being played, triggered by MIDI note-on.

use crate::slicer::{FilterMode, RetriggerRate, SliceLoop};

const MAX_VOICES: usize = 32;

/// State Variable Filter (SVF) per channel for smooth resonance & stability.
#[derive(Clone, Default)]
struct SvfState {
    ic1eq: f32,
    ic2eq: f32,
}

impl SvfState {
    #[inline(always)]
    fn process(
        &mut self,
        input: f32,
        mode: FilterMode,
        cutoff_hz: f32,
        resonance_q: f32,
        sample_rate: f32,
    ) -> f32 {
        if mode == FilterMode::Off {
            return input;
        }
        let cutoff_clamped = cutoff_hz.clamp(20.0, sample_rate * 0.48);
        let g = (std::f32::consts::PI * cutoff_clamped / sample_rate).tan();
        let k = 1.0 / resonance_q.max(0.5);

        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;

        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        match mode {
            FilterMode::Lowpass => v2,
            FilterMode::Highpass => input - k * v1 - v2,
            FilterMode::Bandpass => v1,
            FilterMode::Off => input,
        }
    }
}

#[inline(always)]
fn bitcrush(sample: f32, bits: f32) -> f32 {
    if bits >= 15.9 {
        return sample;
    }
    let levels = (2.0f32).powf(bits.clamp(1.0, 16.0));
    (sample * levels).round() / levels
}

#[inline(always)]
fn apply_drive(sample: f32, drive: f32) -> f32 {
    if drive <= 0.001 {
        return sample;
    }
    let gain = 1.0 + drive * 4.0;
    let x = sample * gain;
    x / (1.0 + x.abs())
}

#[derive(Clone)]
struct Voice {
    active: bool,
    slice_idx: usize,
    /// Read position in frames (fractional for pitch shift).
    playhead: f64,
    /// Playback rate (1.0 = normal, 2.0 = +1 octave).
    rate: f64,
    velocity: f32,
    #[allow(dead_code)]
    note_id: i32,
    reverse: bool,
    gain: f32,
    pan: f32,

    // Per-voice DSP states
    filter_svf_l: SvfState,
    filter_svf_r: SvfState,
    downsample_counter: u32,
    held_sample_l: f32,
    held_sample_r: f32,

    // Retrigger state
    retrigger_frame_count: usize,
    retrigger_gain_mult: f32,

    // DJM500 / Oldschool Jungle Dub Echo delay state (per voice)
    delay_line_l: Vec<f32>,
    delay_line_r: Vec<f32>,
    delay_write_pos: usize,
    delay_filter_l: f32,
    delay_filter_r: f32,
}

#[derive(Clone, Default)]
struct MasterFxState {
    downsample_counter: u32,
    held_sample_l: f32,
    held_sample_r: f32,

    // 6-pole Butterworth/S950 filter states (3 cascaded 2-pole SVFs per channel)
    s950_svf_l1: SvfState,
    s950_svf_r1: SvfState,
    s950_svf_l2: SvfState,
    s950_svf_r2: SvfState,
    s950_svf_l3: SvfState,
    s950_svf_r3: SvfState,

    // Tape saturation warmth EQ state (low-pass 75Hz head bump)
    tape_bump_svf_l: SvfState,
    tape_bump_svf_r: SvfState,
    // Tape softness HF roll-off state
    tape_soft_svf_l: SvfState,
    tape_soft_svf_r: SvfState,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            active: false,
            slice_idx: 0,
            playhead: 0.0,
            rate: 1.0,
            velocity: 1.0,
            note_id: -1,
            reverse: false,
            gain: 1.0,
            pan: 0.0,
            filter_svf_l: SvfState::default(),
            filter_svf_r: SvfState::default(),
            downsample_counter: 0,
            held_sample_l: 0.0,
            held_sample_r: 0.0,
            retrigger_frame_count: 0,
            retrigger_gain_mult: 1.0,
            delay_line_l: vec![0.0; 96000],
            delay_line_r: vec![0.0; 96000],
            delay_write_pos: 0,
            delay_filter_l: 0.0,
            delay_filter_r: 0.0,
        }
    }
}

pub struct Engine {
    voices: [Voice; MAX_VOICES],
    /// Output gain (0.0–2.0).
    pub master_gain: f32,
    master_fx_state: MasterFxState,
    /// Full loop preview playhead position in frames.
    preview_playhead: Option<usize>,
    /// File explorer audition buffer: (interleaved stereo pcm, read frame head).
    audition_buffer: Option<(Vec<f32>, usize)>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            voices: std::array::from_fn(|_| Voice::default()),
            master_gain: 1.0,
            master_fx_state: MasterFxState::default(),
            preview_playhead: None,
            audition_buffer: None,
        }
    }

    fn process_master_fx(&mut self, output: &mut [f32], frames: usize, sample_rate: f32, params: &crate::slicer::MasterFxParams) {
        if !params.s950_enabled && !params.tape_enabled {
            return;
        }

        let ds_ratio = (sample_rate / params.s950_rate_hz.clamp(1000.0, sample_rate)).max(1.0);
        let ds_factor = ds_ratio.round() as u32;

        for f in 0..frames {
            let mut l = output[f * 2];
            let mut r = output[f * 2 + 1];

            // ── 1. Akai S950 Sampler Emulation ──────────────────────────────
            if params.s950_enabled {
                // Variable Clock Downsampling with Zero-Order Hold (ZOH)
                if ds_factor > 1 {
                    if self.master_fx_state.downsample_counter % ds_factor == 0 {
                        self.master_fx_state.held_sample_l = l;
                        self.master_fx_state.held_sample_r = r;
                    }
                    self.master_fx_state.downsample_counter = self.master_fx_state.downsample_counter.wrapping_add(1);
                    l = self.master_fx_state.held_sample_l;
                    r = self.master_fx_state.held_sample_r;
                }

                // 12-bit / 10-bit DAC Quantization
                l = bitcrush(l, params.s950_bit_depth);
                r = bitcrush(r, params.s950_bit_depth);

                // Steep 6-pole S950 Lowpass Reconstruction Filter (3 cascaded 2-pole SVFs)
                let fc = params.s950_filter_cutoff.clamp(200.0, sample_rate * 0.48);
                l = self.master_fx_state.s950_svf_l1.process(l, FilterMode::Lowpass, fc, 0.707, sample_rate);
                l = self.master_fx_state.s950_svf_l2.process(l, FilterMode::Lowpass, fc, 0.707, sample_rate);
                l = self.master_fx_state.s950_svf_l3.process(l, FilterMode::Lowpass, fc, 0.707, sample_rate);

                r = self.master_fx_state.s950_svf_r1.process(r, FilterMode::Lowpass, fc, 0.707, sample_rate);
                r = self.master_fx_state.s950_svf_r2.process(r, FilterMode::Lowpass, fc, 0.707, sample_rate);
                r = self.master_fx_state.s950_svf_r3.process(r, FilterMode::Lowpass, fc, 0.707, sample_rate);
            }

            // ── 2. Master Saturation / Overdrive Circuit ─────────────────
            if params.tape_enabled {
                match params.sat_mode {
                    crate::slicer::MasterSatMode::Tape => {
                        let drive_gain = 1.0 + params.tape_drive * 3.5;

                        // Low-end tape bump around 75 Hz
                        if params.tape_warmth > 0.01 {
                            let bump_l = self.master_fx_state.tape_bump_svf_l.process(l, FilterMode::Lowpass, 75.0, 1.2, sample_rate);
                            let bump_r = self.master_fx_state.tape_bump_svf_r.process(r, FilterMode::Lowpass, 75.0, 1.2, sample_rate);
                            l += bump_l * (params.tape_warmth * 0.45);
                            r += bump_r * (params.tape_warmth * 0.45);
                        }

                        // Soft 2nd & 3rd Harmonic Tape Saturation Curve (tanh with asymmetric warmth)
                        let xl = l * drive_gain;
                        let xr = r * drive_gain;

                        let sat_l = xl.tanh() + 0.08 * params.tape_warmth * (xl.tanh() * xl.tanh());
                        let sat_r = xr.tanh() + 0.08 * params.tape_warmth * (xr.tanh() * xr.tanh());

                        let comp_gain = 1.0 / (1.0 + params.tape_drive * 0.8);
                        l = sat_l * comp_gain;
                        r = sat_r * comp_gain;

                        // High-frequency tape compression & softness
                        if params.tape_softness > 0.01 {
                            let fc_soft = (16000.0 - params.tape_softness * 8000.0).clamp(1000.0, sample_rate * 0.48);
                            l = self.master_fx_state.tape_soft_svf_l.process(l, FilterMode::Lowpass, fc_soft, 0.707, sample_rate);
                            r = self.master_fx_state.tape_soft_svf_r.process(r, FilterMode::Lowpass, fc_soft, 0.707, sample_rate);
                        }
                    }
                    crate::slicer::MasterSatMode::Digitakt => {
                        // ── Elektron Digitakt Warm Overdrive ─────────────
                        // 1. Soft anti-harshness pre-filter
                        l = self.master_fx_state.tape_soft_svf_l.process(l, FilterMode::Lowpass, 14000.0, 0.707, sample_rate);
                        r = self.master_fx_state.tape_soft_svf_r.process(r, FilterMode::Lowpass, 14000.0, 0.707, sample_rate);

                        // Low-frequency warmth boost around 90 Hz
                        if params.tape_warmth > 0.01 {
                            let bump_l = self.master_fx_state.tape_bump_svf_l.process(l, FilterMode::Lowpass, 90.0, 1.0, sample_rate);
                            let bump_r = self.master_fx_state.tape_bump_svf_r.process(r, FilterMode::Lowpass, 90.0, 1.0, sample_rate);
                            l += bump_l * (params.tape_warmth * 0.35);
                            r += bump_r * (params.tape_warmth * 0.35);
                        }

                        // Smooth Digitakt asymmetric soft-clip curve x / (1 + |x|^1.4)
                        let drive_gain = 1.0 + params.tape_drive * 4.5;
                        let xl = l * drive_gain;
                        let xr = r * drive_gain;

                        let sat_l = xl / (1.0 + xl.abs().powf(1.4));
                        let sat_r = xr / (1.0 + xr.abs().powf(1.4));

                        // Add subtle 2nd harmonic warmth (tubey / Digitakt character)
                        let warm_l = sat_l + 0.06 * params.tape_warmth * (1.0 - (-sat_l.abs()).exp());
                        let warm_r = sat_r + 0.06 * params.tape_warmth * (1.0 - (-sat_r.abs()).exp());

                        // Soft post-smoothing filter (12 kHz roll-off for ultra-warm, non-harsh sound)
                        let fc_soft = (16000.0 - params.tape_softness * 7000.0).clamp(1000.0, sample_rate * 0.48);
                        let comp_gain = 1.0 / (1.0 + params.tape_drive * 0.6);
                        l = self.master_fx_state.tape_soft_svf_l.process(warm_l * comp_gain, FilterMode::Lowpass, fc_soft, 0.707, sample_rate);
                        r = self.master_fx_state.tape_soft_svf_r.process(warm_r * comp_gain, FilterMode::Lowpass, fc_soft, 0.707, sample_rate);
                    }
                    crate::slicer::MasterSatMode::MackieTransistor => {
                        // ── Mackie CR-1604 Transistor Console Drive ("In The Red") ───
                        // Mid-range punch boost (1.5 kHz console transformer resonance)
                        if params.tape_warmth > 0.01 {
                            let mid_l = self.master_fx_state.tape_bump_svf_l.process(l, FilterMode::Bandpass, 1500.0, 1.4, sample_rate);
                            let mid_r = self.master_fx_state.tape_bump_svf_r.process(r, FilterMode::Bandpass, 1500.0, 1.4, sample_rate);
                            l += mid_l * (params.tape_warmth * 0.40);
                            r += mid_r * (params.tape_warmth * 0.40);
                        }

                        // Symmetrical Transistor Cubic Soft Knee Clipper
                        let drive_gain = 1.0 + params.tape_drive * 6.5;
                        let xl = l * drive_gain;
                        let xr = r * drive_gain;

                        let sat_l = if xl.abs() < 1.0 { xl - (xl * xl * xl) / 3.0 } else { xl.signum() * (2.0 / 3.0) };
                        let sat_r = if xr.abs() < 1.0 { xr - (xr * xr * xr) / 3.0 } else { xr.signum() * (2.0 / 3.0) };

                        // Dynamic compensation gain so output punches without clipping
                        let comp_gain = 1.25 / (1.0 + params.tape_drive * 0.9);
                        l = sat_l * comp_gain;
                        r = sat_r * comp_gain;

                        // High frequency tone control
                        if params.tape_softness > 0.01 {
                            let fc_soft = (18000.0 - params.tape_softness * 9000.0).clamp(1000.0, sample_rate * 0.48);
                            l = self.master_fx_state.tape_soft_svf_l.process(l, FilterMode::Lowpass, fc_soft, 0.707, sample_rate);
                            r = self.master_fx_state.tape_soft_svf_r.process(r, FilterMode::Lowpass, fc_soft, 0.707, sample_rate);
                        }
                    }
                }
            }

            output[f * 2] = l;
            output[f * 2 + 1] = r;
        }
    }

    pub fn play_preview(&mut self) {
        self.preview_playhead = Some(0);
        self.audition_buffer = None;
    }

    pub fn stop_preview(&mut self) {
        self.preview_playhead = None;
    }

    pub fn is_previewing(&self) -> bool {
        self.preview_playhead.is_some()
    }

    pub fn play_audition(&mut self, pcm: Vec<f32>) {
        self.preview_playhead = None;
        self.audition_buffer = Some((pcm, 0));
    }

    pub fn stop_audition(&mut self) {
        self.audition_buffer = None;
    }

    pub fn reset_playback(&mut self) {
        self.preview_playhead = None;
        self.audition_buffer = None;
        for v in self.voices.iter_mut() { v.active = false; }
    }

    #[allow(dead_code)]
    pub fn is_auditioning(&self) -> bool {
        self.audition_buffer.is_some()
    }

    /// Start or steal a voice for `midi_note`.
    pub fn note_on(
        &mut self,
        loop_data: &SliceLoop,
        midi_note: u8,
        velocity: f32,
        _note_id: i32,
    ) {
        // Stop UI preview loop playback when incoming MIDI notes arrive!
        self.preview_playhead = None;
        self.audition_buffer = None;

        // Find which slice this note maps to.
        let Some(slice_idx) = loop_data.slices.iter().position(|s| s.note == midi_note) else { return };
        let slice = &loop_data.slices[slice_idx];
        let clamped_end = slice.end.min(loop_data.loop_end);
        let slice_frames = clamped_end.saturating_sub(slice.start);
        if slice.muted || slice_frames == 0 { return; }

        let choke_group = slice.fx.choke_group;

        // Retrigger choke & Choke Groups:
        // 1. Stop any active voice playing this exact slice.
        // 2. Stop any active voice whose slice has a retrigger rate active (retriggered slice gets choked by next slice!).
        // 3. Stop any active voice belonging to the same choke group.
        for v in self.voices.iter_mut() {
            if v.active {
                if v.slice_idx == slice_idx {
                    v.active = false;
                } else if let Some(other_slice) = loop_data.slices.get(v.slice_idx) {
                    if other_slice.fx.retrigger_rate != RetriggerRate::Off {
                        v.active = false;
                    } else if choke_group > 0 && other_slice.fx.choke_group == choke_group {
                        v.active = false;
                    }
                }
            }
        }

        // Pitch shift: semitones → playback rate.
        let rate = 2f64.powf(slice.pitch_semitones as f64 / 12.0);
        let playhead = if slice.reverse { (slice_frames as f64) - 1.0 } else { 0.0 };

        // Find a free voice, or steal the voice furthest along in playback.
        let idx = self.voices.iter().position(|v| !v.active)
            .unwrap_or_else(|| {
                self.voices.iter().enumerate().max_by(|(_, a), (_, b)| {
                    a.playhead.partial_cmp(&b.playhead).unwrap_or(std::cmp::Ordering::Equal)
                }).map(|(i, _)| i).unwrap_or(0)
            });
        let v = &mut self.voices[idx];

        // Reset delay buffer without re-allocating
        for sample in v.delay_line_l.iter_mut() { *sample = 0.0; }
        for sample in v.delay_line_r.iter_mut() { *sample = 0.0; }

        v.active = true;
        v.slice_idx = slice_idx;
        v.playhead = playhead;
        v.rate = rate;
        v.velocity = velocity;
        v.note_id = _note_id;
        v.reverse = slice.reverse;
        v.gain = slice.gain;
        v.pan = slice.pan;
        v.filter_svf_l = SvfState::default();
        v.filter_svf_r = SvfState::default();
        v.downsample_counter = 0;
        v.held_sample_l = 0.0;
        v.held_sample_r = 0.0;
        v.retrigger_frame_count = 0;
        v.retrigger_gain_mult = 1.0;
        v.delay_write_pos = 0;
        v.delay_filter_l = 0.0;
        v.delay_filter_r = 0.0;
    }

    pub fn note_off(&mut self, note_id: i32) {
        // For one-shot sample player: just let voices finish naturally.
        let _ = note_id;
    }

    /// Render `frames` samples into `output` (interleaved stereo L/R).
    /// Called from the audio thread — no allocations.
    pub fn process(
        &mut self,
        output: &mut [f32],
        frames: usize,
        loop_data: &SliceLoop,
    ) {
        // Zero output first.
        for s in output[..frames * 2].iter_mut() { *s = 0.0; }

        // Render loop region preview/playback if active.
        if let Some(ref mut ph) = self.preview_playhead {
            let start = loop_data.loop_start;
            let end = loop_data.loop_end.min(loop_data.total_frames);
            let loop_len = end.saturating_sub(start);

            if loop_len > 0 {
                if *ph < start || *ph >= end {
                    *ph = start;
                }
                for f in 0..frames {
                    let idx = *ph * 2;
                    if idx + 1 < loop_data.audio.len() {
                        output[f * 2]     += loop_data.audio[idx]     * self.master_gain;
                        output[f * 2 + 1] += loop_data.audio[idx + 1] * self.master_gain;
                    }
                    *ph += 1;
                    if *ph >= end {
                        *ph = start;
                    }
                }
            } else {
                self.preview_playhead = None;
            }
        }

        // Render file explorer audition playback if active.
        if let Some((ref audio, ref mut ph)) = self.audition_buffer {
            let total_frames = audio.len() / 2;
            for f in 0..frames {
                if *ph < total_frames {
                    let idx = *ph * 2;
                    if idx + 1 < audio.len() {
                        output[f * 2]     += audio[idx]     * self.master_gain;
                        output[f * 2 + 1] += audio[idx + 1] * self.master_gain;
                    }
                    *ph += 1;
                } else {
                    self.audition_buffer = None;
                    break;
                }
            }
        }

        // Automatic 2.5 ms anti-click micro-ramp for seamless zero-crossing transitions.
        let declick_f = ((loop_data.sample_rate as f32 * 0.0025) as usize).clamp(32, 256);
        let sample_rate = loop_data.sample_rate as f32;
        let bpm = if loop_data.bpm > 0.0 { loop_data.bpm as f32 } else { 120.0 };

        for voice in self.voices.iter_mut() {
            if !voice.active { continue; }
            let Some(slice) = loop_data.slices.get(voice.slice_idx) else {
                voice.active = false;
                continue;
            };
            let clamped_end = slice.end.min(loop_data.loop_end);
            let slice_frames = clamped_end.saturating_sub(slice.start);
            if slice_frames == 0 { voice.active = false; continue; }

            let fade_in_f = slice.fade_in_frames(loop_data.sample_rate).max(declick_f);
            let fade_out_f = slice.fade_out_frames(loop_data.sample_rate).max(declick_f);

            let base_left  = voice.velocity * voice.gain * self.master_gain * (1.0 - voice.pan.max(0.0));
            let base_right = voice.velocity * voice.gain * self.master_gain * (1.0 + voice.pan.min(0.0));

            // Retrigger period calculation (BPM synced)
            let retrigger_period_frames = if slice.fx.retrigger_rate != RetriggerRate::Off {
                let div = slice.fx.retrigger_rate.division_factor() as f32; // e.g. 16 for 1/16
                let beat_sec = 60.0 / bpm;
                let bar_sec = beat_sec * 4.0;
                let period_sec = bar_sec / div;
                (period_sec * sample_rate) as usize
            } else {
                0
            };

            for f in 0..frames {
                // Retrigger check
                if retrigger_period_frames > 0 {
                    if voice.retrigger_frame_count >= retrigger_period_frames {
                        voice.retrigger_frame_count = 0;
                        voice.playhead = if voice.reverse { (slice_frames as f64) - 1.0 } else { 0.0 };
                        voice.retrigger_gain_mult *= (1.0 - slice.fx.retrigger_decay).clamp(0.0, 1.0);
                    }
                    voice.retrigger_frame_count += 1;
                }

                let ph_floor = voice.playhead as usize;
                let frac = voice.playhead - ph_floor as f64;

                // ── Akai Granular Vintage Timestretcher ────────────────────────
                let (mut sl, mut sr) = if (slice.fx.stretch_factor - 1.0).abs() > 0.01 && slice.fx.stretch_factor > 0.05 {
                    let s_factor = slice.fx.stretch_factor as f64;
                    let grain_frames = (slice.fx.stretch_grain_ms * 0.001 * sample_rate).max(100.0) as f64;

                    let norm_pos = voice.playhead / s_factor;
                    let ph1 = norm_pos as usize;
                    let frac1 = norm_pos - ph1 as f64;
                    let (a0l, a0r) = sample_at(loop_data, slice, ph1);
                    let (a1l, a1r) = sample_at(loop_data, slice, ph1 + 1);
                    let g1l = a0l + (a1l - a0l) * frac1 as f32;
                    let g1r = a0r + (a1r - a0r) * frac1 as f32;

                    let phase = (voice.playhead / grain_frames).fract();
                    let win = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * phase).cos()) as f32;

                    let ph2 = (norm_pos + grain_frames * 0.5) as usize;
                    let frac2 = (norm_pos + grain_frames * 0.5) - ph2 as f64;
                    let (b0l, b0r) = sample_at(loop_data, slice, ph2);
                    let (b1l, b1r) = sample_at(loop_data, slice, ph2 + 1);
                    let g2l = b0l + (b1l - b0l) * frac2 as f32;
                    let g2r = b0r + (b1r - b0r) * frac2 as f32;

                    let raw_l = g1l * win + g2l * (1.0 - win);
                    let raw_r = g1r * win + g2r * (1.0 - win);
                    (raw_l, raw_r)
                } else {
                    let (s0l, s0r) = sample_at(loop_data, slice, ph_floor);
                    let (s1l, s1r) = sample_at(loop_data, slice, ph_floor + 1);
                    (s0l + (s1l - s0l) * frac as f32, s0r + (s1r - s0r) * frac as f32)
                };

                // 1. Soft Clipping Drive
                sl = apply_drive(sl, slice.fx.drive);
                sr = apply_drive(sr, slice.fx.drive);

                // 2. Bitcrusher (Downsampling & Bit Depth)
                if slice.fx.downsample_factor > 1 {
                    if voice.downsample_counter % slice.fx.downsample_factor == 0 {
                        voice.held_sample_l = sl;
                        voice.held_sample_r = sr;
                    }
                    voice.downsample_counter += 1;
                    sl = voice.held_sample_l;
                    sr = voice.held_sample_r;
                }
                sl = bitcrush(sl, slice.fx.bit_depth);
                sr = bitcrush(sr, slice.fx.bit_depth);

                // 3. State Variable Filter (SVF - DJM Combo & Standard Filter)
                let (f_mode, f_cutoff) = slice.fx.effective_filter();
                if f_mode != FilterMode::Off {
                    sl = voice.filter_svf_l.process(
                        sl,
                        f_mode,
                        f_cutoff,
                        slice.fx.filter_resonance,
                        sample_rate,
                    );
                    sr = voice.filter_svf_r.process(
                        sr,
                        f_mode,
                        f_cutoff,
                        slice.fx.filter_resonance,
                        sample_rate,
                    );
                }

                // Volume envelope (fade-in & fade-out + anti-click micro-ramp + retrigger decay)
                let mut env_gain = voice.retrigger_gain_mult;
                let ph_frame = if voice.reverse {
                    slice_frames.saturating_sub(1).saturating_sub(ph_floor)
                } else {
                    ph_floor
                };

                if ph_frame < fade_in_f {
                    env_gain *= (ph_frame as f32 / fade_in_f as f32).clamp(0.0, 1.0);
                }
                if ph_frame + fade_out_f >= slice_frames {
                    let rem = slice_frames.saturating_sub(ph_frame);
                    env_gain *= (rem as f32 / fade_out_f as f32).clamp(0.0, 1.0);
                }

                // 4. DJM500 / Oldschool Jungle Dub Echo Delay
                if slice.fx.delay_rate != crate::slicer::DelayRate::Off {
                    let delay_samples = if slice.fx.delay_rate == crate::slicer::DelayRate::Ms {
                        ((slice.fx.delay_ms * 0.001) * sample_rate).round() as usize
                    } else {
                        let beats = slice.fx.delay_rate.beats();
                        ((60.0 / bpm) * beats * sample_rate).round() as usize
                    };
                    let delay_samples = delay_samples.clamp(1, 95999);

                    let read_idx = (voice.delay_write_pos + 96000 - delay_samples) % 96000;
                    let delay_l = voice.delay_line_l[read_idx];
                    let delay_r = voice.delay_line_r[read_idx];

                    // Lowpass 1-pole filter in feedback loop for dub warmth & tone damping
                    let tone_cutoff = slice.fx.delay_tone.clamp(200.0, 12000.0);
                    let alpha = (2.0 * std::f32::consts::PI * tone_cutoff / sample_rate).clamp(0.01, 0.99);
                    voice.delay_filter_l += alpha * (delay_l - voice.delay_filter_l);
                    voice.delay_filter_r += alpha * (delay_r - voice.delay_filter_r);

                    let fb = slice.fx.delay_feedback.clamp(0.0, 0.90);
                    let fb_l = voice.delay_filter_l * fb;
                    let fb_r = voice.delay_filter_r * fb;

                    let dry_l = sl * base_left * env_gain;
                    let dry_r = sr * base_right * env_gain;

                    voice.delay_line_l[voice.delay_write_pos] = dry_l + fb_l;
                    voice.delay_line_r[voice.delay_write_pos] = dry_r + fb_r;
                    voice.delay_write_pos = (voice.delay_write_pos + 1) % 96000;

                    let mix = slice.fx.delay_mix.clamp(0.0, 1.0);
                    output[f * 2]     += dry_l * (1.0 - mix * 0.5) + delay_l * mix;
                    output[f * 2 + 1] += dry_r * (1.0 - mix * 0.5) + delay_r * mix;
                } else {
                    output[f * 2]     += sl * base_left * env_gain;
                    output[f * 2 + 1] += sr * base_right * env_gain;
                }

                // Advance playhead.
                if voice.reverse {
                    voice.playhead -= voice.rate;
                    if voice.playhead < 0.0 { voice.active = false; break; }
                } else {
                    voice.playhead += voice.rate;
                    if voice.playhead >= slice_frames as f64 { voice.active = false; break; }
                }
            }
        }

        // Apply Global Master Audio FX (Akai S950 & Analog Tape Saturation)
        self.process_master_fx(output, frames, sample_rate, &loop_data.master_fx);
    }

    #[allow(dead_code)]
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    pub fn active_slice_indices(&self) -> Vec<usize> {
        let mut indices = Vec::new();
        for voice in &self.voices {
            if voice.active {
                indices.push(voice.slice_idx);
            }
        }
        indices
    }

    pub fn active_playhead_frames(&self, loop_data: &SliceLoop) -> Vec<usize> {
        let mut frames = Vec::new();
        if let Some(ph) = self.preview_playhead {
            frames.push(ph);
        }
        for voice in &self.voices {
            if voice.active {
                if let Some(slice) = loop_data.slices.get(voice.slice_idx) {
                    let frame = slice.start + (voice.playhead as usize);
                    frames.push(frame);
                }
            }
        }
        frames
    }
}

#[inline(always)]
fn sample_at(loop_data: &SliceLoop, slice: &crate::slicer::Slice, offset: usize) -> (f32, f32) {
    let raw_frame = slice.start + offset;
    let frame = raw_frame
        .min(loop_data.loop_end.saturating_sub(1))
        .min(loop_data.total_frames.saturating_sub(1));
    if frame * 2 + 1 < loop_data.audio.len() {
        (loop_data.audio[frame * 2], loop_data.audio[frame * 2 + 1])
    } else {
        (0.0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::{FilterMode, RetriggerRate, Slice, SliceFx, SliceLoop};

    #[test]
    fn test_retrigger_choked_by_next_slice() {
        let mut sl = SliceLoop {
            file_path: None,
            audio: vec![0.5; 88200],
            channels: 2,
            sample_rate: 44100,
            total_frames: 44100,
            loop_start: 0,
            loop_end: 44100,
            bpm: 174.0,
            slices: Vec::new(),
            master_fx: crate::slicer::MasterFxParams::default(),
            peak_cache: Vec::new(),
        };

        let mut s1 = Slice::new(0, 20000, 60);
        s1.fx.retrigger_rate = RetriggerRate::Sixteenth;
        sl.slices.push(s1);

        let s2 = Slice::new(20000, 40000, 61);
        sl.slices.push(s2);

        let mut engine = Engine::new();

        // Note 60 (Slice #1) starts
        engine.note_on(&sl, 60, 1.0, 1);
        assert_eq!(engine.voices.iter().filter(|v| v.active).count(), 1);
        assert_eq!(engine.voices.iter().find(|v| v.active).unwrap().slice_idx, 0);

        // Note 61 (Slice #2) triggers -> Slice #1 (retriggered) MUST be choked!
        engine.note_on(&sl, 61, 1.0, 2);
        let active_voices: Vec<_> = engine.voices.iter().filter(|v| v.active).collect();
        assert_eq!(active_voices.len(), 1);
        assert_eq!(active_voices[0].slice_idx, 1);
    }

    #[test]
    fn test_akai_timestretch_dsp() {
        let mut sl = SliceLoop {
            file_path: None,
            audio: vec![0.8; 88200],
            channels: 2,
            sample_rate: 44100,
            total_frames: 44100,
            loop_start: 0,
            loop_end: 44100,
            bpm: 174.0,
            slices: Vec::new(),
            master_fx: crate::slicer::MasterFxParams::default(),
            peak_cache: Vec::new(),
        };
        let mut s1 = Slice::new(0, 20000, 60);
        s1.fx.stretch_factor = 1.5;
        s1.fx.stretch_grain_ms = 40.0;
        sl.slices.push(s1);

        let mut engine = Engine::new();
        engine.note_on(&sl, 60, 1.0, 1);

        let mut output = vec![0.0f32; 1024];
        engine.process(&mut output, 512, &sl);

        let sum: f32 = output.iter().map(|s| s.abs()).sum();
        assert!(sum > 0.0, "Akai timestretcher rendered silent audio");
    }

    #[test]
    fn test_jungle_dub_echo_dsp() {
        let mut sl = SliceLoop {
            file_path: None,
            audio: vec![0.8; 88200],
            channels: 2,
            sample_rate: 44100,
            total_frames: 44100,
            loop_start: 0,
            loop_end: 44100,
            bpm: 174.0,
            slices: Vec::new(),
            master_fx: crate::slicer::MasterFxParams::default(),
            peak_cache: Vec::new(),
        };
        let mut s1 = Slice::new(0, 1000, 60);
        s1.fx.delay_rate = crate::slicer::DelayRate::Sixteenth;
        s1.fx.delay_feedback = 0.5;
        s1.fx.delay_mix = 0.4;
        sl.slices.push(s1);

        let mut engine = Engine::new();
        engine.note_on(&sl, 60, 1.0, 1);

        let mut output = vec![0.0f32; 8192];
        engine.process(&mut output, 4096, &sl);

        let sum: f32 = output.iter().map(|s| s.abs()).sum();
        assert!(sum > 0.0, "Jungle Dub Echo rendered silent audio");
    }

    #[test]
    fn test_jungle_dub_echo_free_ms_dsp() {
        let mut sl = SliceLoop {
            file_path: None,
            audio: vec![0.8; 88200],
            channels: 2,
            sample_rate: 44100,
            total_frames: 44100,
            loop_start: 0,
            loop_end: 44100,
            bpm: 174.0,
            slices: Vec::new(),
            master_fx: crate::slicer::MasterFxParams::default(),
            peak_cache: Vec::new(),
        };
        let mut s1 = Slice::new(0, 1000, 60);
        s1.fx.delay_rate = crate::slicer::DelayRate::Ms;
        s1.fx.delay_ms = 15.0;
        s1.fx.delay_feedback = 0.6;
        s1.fx.delay_mix = 0.5;
        sl.slices.push(s1);

        let mut engine = Engine::new();
        engine.note_on(&sl, 60, 1.0, 1);

        let mut output = vec![0.0f32; 4096];
        engine.process(&mut output, 2048, &sl);

        let sum: f32 = output.iter().map(|s| s.abs()).sum();
        assert!(sum > 0.0, "Jungle Dub Echo free ms rendered silent audio");
    }

    #[test]
    fn test_slice_note_change_and_playback() {
        let mut sl = SliceLoop {
            file_path: None,
            audio: vec![0.5; 88200],
            channels: 2,
            sample_rate: 44100,
            total_frames: 44100,
            loop_start: 0,
            loop_end: 44100,
            bpm: 174.0,
            slices: Vec::new(),
            master_fx: crate::slicer::MasterFxParams::default(),
            peak_cache: Vec::new(),
        };
        let s1 = Slice::new(0, 5000, 48);
        let s2 = Slice::new(5000, 10000, 49);
        sl.slices.push(s1);
        sl.slices.push(s2);

        // Swap notes between slice 0 and slice 1
        sl.slices[0].note = 49;
        sl.slices[1].note = 48;

        let mut engine = Engine::new();
        engine.note_on(&sl, 48, 1.0, 1);

        let active_voice = engine.voices.iter().find(|v| v.active).expect("Voice should be active");
        assert_eq!(active_voice.slice_idx, 1);

        let mut output = vec![0.0f32; 1024];
        engine.process(&mut output, 512, &sl);

        let sum: f32 = output.iter().map(|s| s.abs()).sum();
        assert!(sum > 0.0, "Changed slice note rendered silent audio");
    }

    #[test]
    fn test_slice_note_change_back_and_forth() {
        let mut sl = SliceLoop {
            file_path: None,
            audio: vec![0.5; 88200],
            channels: 2,
            sample_rate: 44100,
            total_frames: 44100,
            loop_start: 0,
            loop_end: 44100,
            bpm: 174.0,
            slices: (0..10).map(|i| Slice::new(i * 1000, (i + 1) * 1000, 48 + i as u8)).collect(),
            master_fx: crate::slicer::MasterFxParams::default(),
            peak_cache: Vec::new(),
        };

        // Slice 5 (6th slice) starts at F2 (53)
        assert_eq!(sl.slices[5].note, 53);

        // Change slice 5 from F2 (53) to G2 (55)
        sl.set_slice_note(5, 55);
        assert_eq!(sl.slices[5].note, 55);
        assert_eq!(sl.slices[7].note, 53);

        // Change slice 5 back from G2 (55) to F2 (53)
        sl.set_slice_note(5, 53);
        assert_eq!(sl.slices[5].note, 53);
        assert_eq!(sl.slices[7].note, 55);

        // Play F2 (53)
        let mut engine = Engine::new();
        engine.note_on(&sl, 53, 1.0, 1);

        let active_voice = engine.voices.iter().find(|v| v.active).expect("Voice should be active");
        assert_eq!(active_voice.slice_idx, 5, "Triggered slice should be slice 5");

        let mut output = vec![0.0f32; 1024];
        engine.process(&mut output, 512, &sl);

        let sum: f32 = output.iter().map(|s| s.abs()).sum();
        assert!(sum > 0.0, "Changed-back slice note rendered silent audio");
    }

    #[test]
    fn test_djm_combo_filter_effective() {
        let mut fx = SliceFx::default();
        assert_eq!(fx.effective_filter(), (FilterMode::Off, 20000.0));

        fx.filter_djm = -0.5;
        let (mode_lp, cutoff_lp) = fx.effective_filter();
        assert_eq!(mode_lp, FilterMode::Lowpass);
        assert!(cutoff_lp > 100.0 && cutoff_lp < 5000.0);

        fx.filter_djm = 0.5;
        let (mode_hp, cutoff_hp) = fx.effective_filter();
        assert_eq!(mode_hp, FilterMode::Highpass);
        assert!(cutoff_hp > 100.0 && cutoff_hp < 5000.0);
    }

    #[test]
    fn test_akai_s950_and_tape_master_fx_dsp() {
        let sl = SliceLoop {
            file_path: None,
            audio: vec![0.6; 88200],
            channels: 2,
            sample_rate: 44100,
            total_frames: 44100,
            loop_start: 0,
            loop_end: 44100,
            bpm: 174.0,
            slices: vec![Slice::new(0, 10000, 48)],
            master_fx: crate::slicer::MasterFxParams {
                s950_enabled: true,
                s950_rate_hz: 12000.0,
                s950_bit_depth: 12.0,
                s950_filter_cutoff: 8000.0,
                tape_enabled: true,
                sat_mode: crate::slicer::MasterSatMode::Tape,
                tape_drive: 0.40,
                tape_warmth: 0.50,
                tape_softness: 0.35,
            },
            peak_cache: Vec::new(),
        };

        let mut engine = Engine::new();
        engine.note_on(&sl, 48, 1.0, 1);

        let mut output = vec![0.0f32; 2048];
        engine.process(&mut output, 1024, &sl);

        let sum: f32 = output.iter().map(|s| s.abs()).sum();
        assert!(sum > 0.0, "Akai S950 & Tape Saturation rendered silent audio");
    }

    #[test]
    fn test_digitakt_and_mackie_master_saturation_dsp() {
        for mode in [crate::slicer::MasterSatMode::Digitakt, crate::slicer::MasterSatMode::MackieTransistor] {
            let sl = SliceLoop {
                file_path: None,
                audio: vec![0.7; 88200],
                channels: 2,
                sample_rate: 44100,
                total_frames: 44100,
                loop_start: 0,
                loop_end: 44100,
                bpm: 174.0,
                slices: vec![Slice::new(0, 10000, 48)],
                master_fx: crate::slicer::MasterFxParams {
                    s950_enabled: false,
                    s950_rate_hz: 12000.0,
                    s950_bit_depth: 12.0,
                    s950_filter_cutoff: 8000.0,
                    tape_enabled: true,
                    sat_mode: mode,
                    tape_drive: 0.50,
                    tape_warmth: 0.50,
                    tape_softness: 0.30,
                },
                peak_cache: Vec::new(),
            };

            let mut engine = Engine::new();
            engine.note_on(&sl, 48, 1.0, 1);

            let mut output = vec![0.0f32; 2048];
            engine.process(&mut output, 1024, &sl);

            let sum: f32 = output.iter().map(|s| s.abs()).sum();
            assert!(sum > 0.0, "Master Saturation mode {:?} rendered silent audio", mode);
        }
    }
}
