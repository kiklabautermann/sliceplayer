//! Polyphonic audio engine.
//! One Voice per slice being played, triggered by MIDI note-on.

use crate::slicer::SliceLoop;

const MAX_VOICES: usize = 32;

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
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            active: false, slice_idx: 0, playhead: 0.0,
            rate: 1.0, velocity: 1.0, note_id: -1,
            reverse: false, gain: 1.0, pan: 0.0,
        }
    }
}

pub struct Engine {
    voices: [Voice; MAX_VOICES],
    /// Output gain (0.0–2.0).
    pub master_gain: f32,
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
            preview_playhead: None,
            audition_buffer: None,
        }
    }

    pub fn play_preview(&mut self) {
        self.preview_playhead = Some(0);
    }

    pub fn stop_preview(&mut self) {
        self.preview_playhead = None;
    }

    pub fn is_previewing(&self) -> bool {
        self.preview_playhead.is_some()
    }

    pub fn play_audition(&mut self, stereo_pcm: Vec<f32>) {
        self.audition_buffer = Some((stereo_pcm, 0));
    }

    pub fn stop_audition(&mut self) {
        self.audition_buffer = None;
    }

    pub fn reset_playback(&mut self) {
        self.audition_buffer = None;
        self.preview_playhead = None;
        for v in self.voices.iter_mut() {
            v.active = false;
        }
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

        // Retrigger choke: stop any active voice already playing this exact slice!
        for v in self.voices.iter_mut() {
            if v.active && v.slice_idx == slice_idx {
                v.active = false;
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

        *v = Voice {
            active: true, slice_idx, playhead, rate, velocity,
            note_id: _note_id, reverse: slice.reverse, gain: slice.gain, pan: slice.pan,
        };
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

            for f in 0..frames {
                // Linear interpolation between adjacent frames.
                let ph_floor = voice.playhead as usize;
                let frac = voice.playhead - ph_floor as f64;

                let (s0l, s0r) = sample_at(loop_data, slice, ph_floor);
                let (s1l, s1r) = sample_at(loop_data, slice, ph_floor + 1);

                let sl = s0l + (s1l - s0l) * frac as f32;
                let sr = s0r + (s1r - s0r) * frac as f32;

                // Volume envelope (fade-in & fade-out + anti-click micro-ramp)
                let mut env_gain = 1.0f32;
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

                output[f * 2]     += sl * base_left * env_gain;
                output[f * 2 + 1] += sr * base_right * env_gain;

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
    }

    #[allow(dead_code)]
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
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
