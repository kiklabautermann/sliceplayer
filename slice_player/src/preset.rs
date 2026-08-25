//! Full Preset Save & Load (.sliceplayer / .json) for SlicePlayer.

use std::path::Path;
use crate::SlicePersisted;
use crate::slicer::{Slice, SliceLoop, FilterMode, RetriggerRate};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct SlicePlayerPreset {
    pub version: u32,
    pub file_path: Option<std::path::PathBuf>,
    pub bpm: f64,
    pub sample_rate: u32,
    pub loop_start: usize,
    pub loop_end: usize,
    pub slices: Vec<SlicePersisted>,
    pub audio_pcm: Vec<f32>,
}

pub fn save_preset_to_file(path: &Path, sl: &SliceLoop) -> Result<(), String> {
    let slices: Vec<SlicePersisted> = sl.slices.iter().map(|s| SlicePersisted {
        start: s.start, end: s.end, note: s.note, gain: s.gain,
        pan: s.pan, pitch_semitones: s.pitch_semitones,
        reverse: s.reverse, muted: s.muted,
        fade_in_ms: s.fade_in_ms, fade_out_ms: s.fade_out_ms,
        filter_mode: match s.fx.filter_mode {
            FilterMode::Off => 0,
            FilterMode::Lowpass => 1,
            FilterMode::Highpass => 2,
            FilterMode::Bandpass => 3,
        },
        filter_cutoff: s.fx.filter_cutoff,
        filter_resonance: s.fx.filter_resonance,
        bit_depth: s.fx.bit_depth,
        downsample_factor: s.fx.downsample_factor,
        drive: s.fx.drive,
        retrigger_rate: match s.fx.retrigger_rate {
            RetriggerRate::Off => 0,
            RetriggerRate::Eighth => 1,
            RetriggerRate::Sixteenth => 2,
            RetriggerRate::ThirtySecond => 3,
            RetriggerRate::SixtyFourth => 4,
        },
        retrigger_decay: s.fx.retrigger_decay,
        choke_group: s.fx.choke_group,
        stretch_factor: s.fx.stretch_factor,
        stretch_grain_ms: s.fx.stretch_grain_ms,
        delay_rate: match s.fx.delay_rate {
            crate::slicer::DelayRate::Off => 0,
            crate::slicer::DelayRate::SixtyFourth => 1,
            crate::slicer::DelayRate::ThirtySecond => 2,
            crate::slicer::DelayRate::Sixteenth => 3,
            crate::slicer::DelayRate::Eighth => 4,
            crate::slicer::DelayRate::DottedEighth => 5,
            crate::slicer::DelayRate::Quarter => 6,
            crate::slicer::DelayRate::Half => 7,
        },
        delay_feedback: s.fx.delay_feedback,
        delay_mix: s.fx.delay_mix,
        delay_tone: s.fx.delay_tone,
    }).collect();

    let preset = SlicePlayerPreset {
        version: 1,
        file_path: sl.file_path.clone(),
        bpm: sl.bpm,
        sample_rate: sl.sample_rate,
        loop_start: sl.loop_start,
        loop_end: sl.loop_end,
        slices,
        audio_pcm: sl.audio.clone(),
    };

    let json = serde_json::to_string_pretty(&preset)
        .map_err(|e| format!("Failed to serialize preset: {e}"))?;

    std::fs::write(path, json)
        .map_err(|e| format!("Failed to write preset file: {e}"))?;

    Ok(())
}

pub fn load_preset_from_file(path: &Path) -> Result<SliceLoop, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read preset file: {e}"))?;

    let preset: SlicePlayerPreset = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse preset JSON: {e}"))?;

    let total_frames = preset.audio_pcm.len() / 2;
    if total_frames == 0 {
        return Err("Preset contains no audio data".into());
    }

    let mut restored = SliceLoop {
        file_path: preset.file_path,
        audio: preset.audio_pcm,
        channels: 2,
        sample_rate: preset.sample_rate,
        total_frames,
        loop_start: preset.loop_start.min(total_frames),
        loop_end: if preset.loop_end > 0 { preset.loop_end.min(total_frames) } else { total_frames },
        bpm: preset.bpm,
        slices: Vec::new(),
        peak_cache: Vec::new(),
    };

    restored.slices = preset.slices.iter().map(|s| {
        let mut slice = Slice::new(s.start, s.end, s.note);
        slice.gain = s.gain;
        slice.pan = s.pan;
        slice.pitch_semitones = s.pitch_semitones;
        slice.reverse = s.reverse;
        slice.muted = s.muted;
        slice.fade_in_ms = s.fade_in_ms;
        slice.fade_out_ms = s.fade_out_ms;
        slice.fx.filter_mode = match s.filter_mode {
            1 => FilterMode::Lowpass,
            2 => FilterMode::Highpass,
            3 => FilterMode::Bandpass,
            _ => FilterMode::Off,
        };
        slice.fx.filter_cutoff = if s.filter_cutoff > 0.0 { s.filter_cutoff } else { 20000.0 };
        slice.fx.filter_resonance = if s.filter_resonance > 0.0 { s.filter_resonance } else { 0.707 };
        slice.fx.bit_depth = if s.bit_depth > 0.0 { s.bit_depth } else { 16.0 };
        slice.fx.downsample_factor = s.downsample_factor.max(1);
        slice.fx.drive = s.drive;
        slice.fx.retrigger_rate = match s.retrigger_rate {
            1 => RetriggerRate::Eighth,
            2 => RetriggerRate::Sixteenth,
            3 => RetriggerRate::ThirtySecond,
            4 => RetriggerRate::SixtyFourth,
            _ => RetriggerRate::Off,
        };
        slice.fx.retrigger_decay = s.retrigger_decay;
        slice.fx.choke_group = s.choke_group;
        slice.fx.stretch_factor = if s.stretch_factor > 0.0 { s.stretch_factor } else { 1.0 };
        slice.fx.stretch_grain_ms = if s.stretch_grain_ms > 0.0 { s.stretch_grain_ms } else { 30.0 };
        slice.fx.delay_rate = match s.delay_rate {
            1 => crate::slicer::DelayRate::SixtyFourth,
            2 => crate::slicer::DelayRate::ThirtySecond,
            3 => crate::slicer::DelayRate::Sixteenth,
            4 => crate::slicer::DelayRate::Eighth,
            5 => crate::slicer::DelayRate::DottedEighth,
            6 => crate::slicer::DelayRate::Quarter,
            7 => crate::slicer::DelayRate::Half,
            _ => crate::slicer::DelayRate::Off,
        };
        slice.fx.delay_feedback = s.delay_feedback;
        slice.fx.delay_mix = s.delay_mix;
        slice.fx.delay_tone = if s.delay_tone > 0.0 { s.delay_tone } else { 3500.0 };
        slice
    }).collect();

    restored.rebuild_peaks(1024);
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preset_save_and_load() {
        let sample_rate = 44100;
        let pcm = vec![0.0f32; 88200]; // 1 sec stereo
        let mut sl = SliceLoop {
            file_path: Some(std::path::PathBuf::from("/tmp/test.wav")),
            audio: pcm,
            channels: 2,
            sample_rate,
            total_frames: 44100,
            loop_start: 0,
            loop_end: 44100,
            bpm: 174.0,
            slices: Vec::new(),
            peak_cache: Vec::new(),
        };

        let mut s1 = Slice::new(0, 22050, 60);
        s1.gain = 1.5;
        s1.pan = -0.5;
        s1.pitch_semitones = 3.0;
        s1.fx.filter_mode = FilterMode::Lowpass;
        s1.fx.filter_cutoff = 2500.0;
        s1.fx.filter_resonance = 2.0;
        s1.fx.drive = 0.4;
        s1.fx.retrigger_rate = RetriggerRate::Sixteenth;
        s1.fx.choke_group = 1;
        sl.slices.push(s1);

        let temp_dir = std::env::temp_dir();
        let preset_path = temp_dir.join("test_preset.sliceplayer");

        save_preset_to_file(&preset_path, &sl).expect("Save preset failed");

        let loaded = load_preset_from_file(&preset_path).expect("Load preset failed");

        assert_eq!(loaded.bpm, 174.0);
        assert_eq!(loaded.slices.len(), 1);
        let loaded_s1 = &loaded.slices[0];
        assert_eq!(loaded_s1.note, 60);
        assert_eq!(loaded_s1.gain, 1.5);
        assert_eq!(loaded_s1.fx.filter_mode, FilterMode::Lowpass);
        assert_eq!(loaded_s1.fx.filter_cutoff, 2500.0);
        assert_eq!(loaded_s1.fx.retrigger_rate, RetriggerRate::Sixteenth);
        assert_eq!(loaded_s1.fx.choke_group, 1);

        let _ = std::fs::remove_file(preset_path);
    }
}
