//! MIDI file export (Type 0, 960 ppq) for Bitwig import.
//! Maps each slice to a NoteOn/NoteOff event on its assigned MIDI note.

use std::path::Path;
use crate::slicer::SliceLoop;

/// Write a MIDI Type-0 file from the current SliceLoop slice layout.
pub fn export_midi(loop_data: &SliceLoop, path: &Path) -> Result<(), String> {
    use midly::{
        Format, Header, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
        num::{u15, u28, u4, u7},
        MetaMessage,
    };

    let ppq: u16 = 960;
    let bpm = loop_data.bpm;
    let sample_rate = loop_data.sample_rate as f64;

    // Calculate total beats of loop (rounded to nearest 1/16th beat grid)
    let raw_beats = loop_data.loop_frames() as f64 / sample_rate / (60.0 / bpm);
    let rounded_16ths = (raw_beats * 16.0).round().max(1.0);
    let max_loop_ticks = ((rounded_16ths / 16.0) * 4.0 * ppq as f64).round() as u32;

    // samples → ticks
    let samples_to_ticks = |samples: usize| -> u32 {
        let beats = samples as f64 / sample_rate / (60.0 / bpm);
        (beats * ppq as f64).round() as u32
    };

    let header = Header {
        format: Format::SingleTrack,
        timing: Timing::Metrical(u15::new(ppq)),
    };

    let mut events: Vec<TrackEvent> = Vec::new();
    let channel = u4::new(0);
    let velocity = u7::new(100);

    // Collect all NoteOn/NoteOff events sorted by tick.
    let mut raw: Vec<(u32, bool, u8)> = Vec::new(); // (tick, is_on, note)
    let valid_slices: Vec<(usize, &crate::slicer::Slice)> = loop_data.slices
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            if s.muted { return false; }
            let clamped_start = s.start.max(loop_data.loop_start);
            let clamped_end = s.end.min(loop_data.loop_end);
            clamped_end > clamped_start
        })
        .collect();

    for (_i, (_slice_idx, slice)) in valid_slices.iter().enumerate() {
        let note = slice.note;

        // Clamp slice boundaries to the loop region
        let clamped_start = slice.start.max(loop_data.loop_start);
        let clamped_end = slice.end.min(loop_data.loop_end);

        // Convert offsets relative to loop_start so the exported MIDI clip starts at 0!
        let rel_start = clamped_start - loop_data.loop_start;
        let rel_end = clamped_end - loop_data.loop_start;

        let on_tick  = samples_to_ticks(rel_start).min(max_loop_ticks.saturating_sub(1));
        let off_tick = samples_to_ticks(rel_end).min(max_loop_ticks);

        if off_tick > on_tick {
            raw.push((on_tick,  true,  note));
            raw.push((off_tick, false, note));
        }
    }
    raw.sort_unstable_by_key(|&(t, on, _)| (t, !on as u32));

    // Convert absolute ticks → delta ticks.
    let mut last_tick: u32 = 0;
    for (abs_tick, is_on, note) in raw {
        let delta = abs_tick.saturating_sub(last_tick);
        last_tick = abs_tick;
        let msg = if is_on {
            MidiMessage::NoteOn { key: u7::new(note), vel: velocity }
        } else {
            MidiMessage::NoteOff { key: u7::new(note), vel: u7::new(0) }
        };
        events.push(TrackEvent {
            delta: u28::new(delta),
            kind: TrackEventKind::Midi { channel, message: msg },
        });
    }

    // End-of-track meta at exact loop end tick boundary
    let final_delta = max_loop_ticks.saturating_sub(last_tick);
    events.push(TrackEvent {
        delta: u28::new(final_delta),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let smf = Smf { header, tracks: vec![events] };
    let mut buf: Vec<u8> = Vec::new();
    smf.write(&mut buf).map_err(|e| format!("MIDI write: {e}"))?;
    std::fs::write(path, &buf).map_err(|e| format!("File write: {e}"))?;
    Ok(())
}

/// Set the generated MIDI file onto the OS clipboard as both `text/uri-list`
/// and `audio/midi` using xclip / wl-copy so DAWs like Bitwig can paste (Ctrl+V) it.
pub fn copy_midi_file_to_clipboard(path: &Path) -> Result<(), String> {
    let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let uri = format!("file://{}\r\n", abs_path.display());

    // 1. Try xclip with text/uri-list (standard Freedesktop file clipboard format)
    if let Ok(mut child) = std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "text/uri-list"])
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(uri.as_bytes());
        }
        let _ = child.wait();
        return Ok(());
    }

    // 2. Fallback to wl-copy for Wayland environments
    if let Ok(mut child) = std::process::Command::new("wl-copy")
        .args(["-t", "text/uri-list"])
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(uri.as_bytes());
        }
        let _ = child.wait();
        return Ok(());
    }

    Err("Neither xclip nor wl-copy available".into())
}
