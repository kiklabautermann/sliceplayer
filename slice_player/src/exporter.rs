//! Audio & Loop Exporter for SlicePlayer.
//! Supports:
//! 1. WAV with embedded `cue ` and `acid` chunks (Universal DAW slice WAV).
//! 2. Bitwig Studio `.multisample` archives (ZIP format).

use std::fs::File;
use std::io::Write;
use std::path::Path;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::slicer::SliceLoop;

// ── 1. Export Sliced WAV with Cue & Acid Chunks ──────────────────────────────

pub fn export_sliced_wav(loop_data: &SliceLoop, path: &Path) -> Result<(), String> {
    let mut file = File::create(path).map_err(|e| format!("Create WAV file error: {e}"))?;
    
    let sample_rate = loop_data.sample_rate;
    let channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let block_align = channels * bits_per_sample / 8;
    let byte_rate = sample_rate * block_align as u32;

    // Convert f32 audio -> i16 PCM bytes
    let pcm_i16: Vec<i16> = loop_data.audio
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect();

    let data_bytes_len = (pcm_i16.len() * 2) as u32;

    // Build cue chunk
    let mut cue_payload: Vec<u8> = Vec::new();
    let valid_slices: Vec<&crate::slicer::Slice> = loop_data.slices
        .iter()
        .filter(|s| !s.muted && s.end > loop_data.loop_start && s.start < loop_data.loop_end)
        .collect();

    let num_cues = valid_slices.len() as u32;
    cue_payload.extend_from_slice(&num_cues.to_le_bytes());

    for (i, slice) in valid_slices.iter().enumerate() {
        let cue_id = (i + 1) as u32;
        let pos = (slice.start.max(loop_data.loop_start) - loop_data.loop_start) as u32;
        cue_payload.extend_from_slice(&cue_id.to_le_bytes());        // Cue ID
        cue_payload.extend_from_slice(&pos.to_le_bytes());           // Position
        cue_payload.extend_from_slice(b"data");                      // Data Chunk ID
        cue_payload.extend_from_slice(&0u32.to_le_bytes());          // Chunk Start
        cue_payload.extend_from_slice(&0u32.to_le_bytes());          // Block Start
        cue_payload.extend_from_slice(&pos.to_le_bytes());           // Sample Offset
    }

    // Build acid chunk (Sony Acid Loop chunk, 24 bytes payload)
    let mut acid_payload: Vec<u8> = Vec::new();
    let acid_type: u32 = 1; // Acid Loop
    let root_note: u16 = 60; // C4
    let flags: u16 = 0;
    let num_beats: f32 = loop_data.calculate_bars() as f32 * 4.0;
    let meter_denom: u16 = 4;
    let meter_num: u16 = 4;
    let tempo: f32 = loop_data.bpm as f32;

    acid_payload.extend_from_slice(&acid_type.to_le_bytes());
    acid_payload.extend_from_slice(&root_note.to_le_bytes());
    acid_payload.extend_from_slice(&flags.to_le_bytes());
    acid_payload.extend_from_slice(&0.0f32.to_le_bytes()); // length in beats unscaled
    acid_payload.extend_from_slice(&num_beats.to_le_bytes());
    acid_payload.extend_from_slice(&meter_denom.to_le_bytes());
    acid_payload.extend_from_slice(&meter_num.to_le_bytes());
    acid_payload.extend_from_slice(&tempo.to_le_bytes());

    // Build LIST chunk (type adtl with labl subchunks for slice names)
    let mut adtl_payload: Vec<u8> = Vec::new();
    adtl_payload.extend_from_slice(b"adtl");
    for (i, _slice) in valid_slices.iter().enumerate() {
        let cue_id = (i + 1) as u32;
        let name = format!("Slice {}\0", i + 1);
        let mut name_bytes = name.into_bytes();
        if name_bytes.len() % 2 != 0 {
            name_bytes.push(0); // Pad to word boundary
        }
        let labl_len = 4 + name_bytes.len() as u32;
        adtl_payload.extend_from_slice(b"labl");
        adtl_payload.extend_from_slice(&labl_len.to_le_bytes());
        adtl_payload.extend_from_slice(&cue_id.to_le_bytes());
        adtl_payload.extend_from_slice(&name_bytes);
    }

    // Calculate total RIFF size
    let fmt_chunk_size = 16u32;
    let data_chunk_size = data_bytes_len;
    let cue_chunk_size = cue_payload.len() as u32;
    let acid_chunk_size = acid_payload.len() as u32;
    let list_chunk_size = adtl_payload.len() as u32;

    let riff_payload_size = 4 // 'WAVE'
        + 8 + fmt_chunk_size
        + 8 + data_chunk_size
        + 8 + cue_chunk_size
        + 8 + acid_chunk_size
        + 8 + list_chunk_size;

    // Write RIFF Header
    file.write_all(b"RIFF").map_err(|e| e.to_string())?;
    file.write_all(&riff_payload_size.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(b"WAVE").map_err(|e| e.to_string())?;

    // Write fmt chunk
    file.write_all(b"fmt ").map_err(|e| e.to_string())?;
    file.write_all(&fmt_chunk_size.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&1u16.to_le_bytes()).map_err(|e| e.to_string())?; // PCM
    file.write_all(&channels.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&sample_rate.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&byte_rate.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&block_align.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&bits_per_sample.to_le_bytes()).map_err(|e| e.to_string())?;

    // Write data chunk
    file.write_all(b"data").map_err(|e| e.to_string())?;
    file.write_all(&data_chunk_size.to_le_bytes()).map_err(|e| e.to_string())?;
    for sample in pcm_i16 {
        file.write_all(&sample.to_le_bytes()).map_err(|e| e.to_string())?;
    }

    // Write cue chunk
    file.write_all(b"cue ").map_err(|e| e.to_string())?;
    file.write_all(&cue_chunk_size.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&cue_payload).map_err(|e| e.to_string())?;

    // Write acid chunk
    file.write_all(b"acid").map_err(|e| e.to_string())?;
    file.write_all(&acid_chunk_size.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&acid_payload).map_err(|e| e.to_string())?;

    // Write LIST chunk
    file.write_all(b"LIST").map_err(|e| e.to_string())?;
    file.write_all(&list_chunk_size.to_le_bytes()).map_err(|e| e.to_string())?;
    file.write_all(&adtl_payload).map_err(|e| e.to_string())?;

    Ok(())
}

// ── 2. Export Bitwig Studio .multisample ZIP Archive ─────────────────────────

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
     .replace('\'', "&apos;")
}

fn sanitize_ascii(s: &str) -> String {
    s.chars()
     .map(|c| match c {
         '\u{00A0}' => ' ',
         c if c.is_ascii() && !c.is_ascii_control() => c,
         _ => '_',
     })
     .collect()
}

pub fn export_bitwig_multisample(loop_data: &SliceLoop, path: &Path) -> Result<(), String> {
    let file = File::create(path).map_err(|e| format!("Create .multisample file error: {e}"))?;
    let mut zip = ZipWriter::new(file);
    // Bitwig Spec: ZIP archive must use STORED (uncompressed) method for sample audio streaming without extraction!
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let raw_name = loop_data.file_path.as_ref()
        .and_then(|p| p.file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("SlicePlayer Loop");
    let clean_name = sanitize_ascii(raw_name);
    let name_escaped = xml_escape(&clean_name);

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    xml.push_str(&format!("<multisample name=\"{}\">\n", name_escaped));
    xml.push_str("   <generator>SlicePlayer 1.0</generator>\n");
    xml.push_str("   <category>Sample Slices</category>\n");
    xml.push_str("   <creator>Aura SlicePlayer</creator>\n");
    xml.push_str("   <group color=\"ff8800\" name=\"Slices\"/>\n");

    let sample_rate = loop_data.sample_rate;

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

    if valid_slices.is_empty() {
        return Err("No active slices within loop markers to export.".into());
    }

    for (i, (_slice_idx, slice)) in valid_slices.iter().enumerate() {
        let slice_filename = format!("sample_{:02}.wav", i + 1);
        let note = (48 + i as u8).min(127);

        // Extract slice stereo PCM from loop_data audio (clamped to loop bounds)
        let clamped_start = slice.start.max(loop_data.loop_start);
        let clamped_end = slice.end.min(loop_data.loop_end);

        let slice_frames = clamped_end.saturating_sub(clamped_start);
        if slice_frames == 0 { continue; }

        let start_sample = clamped_start * 2;
        let end_sample = (clamped_end * 2).min(loop_data.audio.len());

        let mut pcm: Vec<f32> = if start_sample < end_sample {
            loop_data.audio[start_sample..end_sample].to_vec()
        } else {
            Vec::new()
        };

        if pcm.is_empty() { continue; }

        // Reverse slice audio frames if reverse option is enabled on slice
        if slice.reverse && pcm.len() >= 2 {
            let frames_count = pcm.len() / 2;
            for f in 0..frames_count / 2 {
                let left_idx = f * 2;
                let right_idx = (frames_count - 1 - f) * 2;
                pcm.swap(left_idx, right_idx);
                pcm.swap(left_idx + 1, right_idx + 1);
            }
        }

        // Apply slice gain & volume fade envelopes
        let fade_in_f = slice.fade_in_frames(sample_rate);
        let fade_out_f = slice.fade_out_frames(sample_rate);

        for f in 0..slice_frames {
            let idx = f * 2;
            if idx + 1 >= pcm.len() { break; }
            let mut env = 1.0f32;
            if fade_in_f > 0 && f < fade_in_f {
                env *= (f as f32 / fade_in_f as f32).clamp(0.0, 1.0);
            }
            if fade_out_f > 0 && f + fade_out_f >= slice_frames {
                let rem = slice_frames.saturating_sub(f);
                env *= (rem as f32 / fade_out_f as f32).clamp(0.0, 1.0);
            }
            pcm[idx]     *= slice.gain * env;
            pcm[idx + 1] *= slice.gain * env;
        }

        let pcm_i16: Vec<i16> = pcm
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
        
        let mut wav_bytes = Vec::new();
        write_simple_wav(&mut wav_bytes, &pcm_i16, sample_rate)?;

        // Add slice WAV to ZIP (STORED / uncompressed method per Bitwig spec)
        zip.start_file(&slice_filename, options).map_err(|e| format!("Zip file add error: {e}"))?;
        zip.write_all(&wav_bytes).map_err(|e| format!("Zip write error: {e}"))?;

        // Add XML entry for slice matching official Bitwig Studio multisample.xml schema
        xml.push_str(&format!(
            "   <sample file=\"{}\" gain=\"0.00\" group=\"0\" parameter-1=\"0.0000\" parameter-2=\"0.0000\" parameter-3=\"0.0000\" reverse=\"false\" sample-start=\"0.000\" sample-stop=\"{}.000\" zone-logic=\"always-play\">\n      <key high=\"{}\" low=\"{}\" root=\"{}\" track=\"0.0000\" tune=\"0.00\"/>\n      <velocity high=\"127\" low=\"0\"/>\n      <select/>\n      <loop fade=\"0.0000\" mode=\"off\" start=\"0.000\" stop=\"{}.000\"/>\n   </sample>\n",
            slice_filename, slice_frames, note, note, note, slice_frames
        ));
    }

    xml.push_str("</multisample>\n");

    // Add multisample.xml to ZIP
    zip.start_file("multisample.xml", options).map_err(|e| format!("Zip XML add error: {e}"))?;
    zip.write_all(xml.as_bytes()).map_err(|e| format!("Zip XML write error: {e}"))?;

    zip.finish().map_err(|e| format!("Zip finish error: {e}"))?;

    // Also export corresponding MIDI clip file with .mid extension into the same directory
    let midi_path = path.with_extension("mid");
    let _ = crate::midi_export::export_midi(loop_data, &midi_path);

    Ok(())
}

fn write_simple_wav(out: &mut Vec<u8>, pcm_i16: &[i16], sample_rate: u32) -> Result<(), String> {
    if pcm_i16.is_empty() {
        return Err("Cannot write WAV file with 0 samples.".into());
    }
    let channels: u16 = 2;
    let bits_per_sample: u16 = 16;
    let block_align = channels * bits_per_sample / 8;
    let byte_rate = sample_rate * block_align as u32;
    let data_bytes_len = (pcm_i16.len() * 2) as u32;
    let riff_size = 36 + data_bytes_len;

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());

    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes_len.to_le_bytes());
    for &sample in pcm_i16 {
        out.extend_from_slice(&sample.to_le_bytes());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slicer::{SliceLoop, Slice};

    #[test]
    fn test_export_bitwig_multisample_stored() {
        let loop_data = SliceLoop {
            file_path: None,
            audio: vec![0.0; 88200], // 1 sec stereo @ 44.1kHz
            channels: 2,
            sample_rate: 44100,
            total_frames: 44100,
            loop_start: 0,
            loop_end: 44100,
            bpm: 120.0,
            slices: vec![
                Slice::new(0, 22050, 48),
                Slice::new(22050, 44100, 49),
            ],
            master_fx: crate::slicer::MasterFxParams::default(),
            peak_cache: Vec::new(),
        };

        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("test_export.multisample");
        export_bitwig_multisample(&loop_data, &test_path).expect("Export failed");

        let file = std::fs::File::open(&test_path).expect("Open export file failed");
        let mut archive = zip::ZipArchive::new(file).expect("Read zip failed");

        assert!(archive.len() > 0);
        for i in 0..archive.len() {
            let entry = archive.by_index(i).expect("Zip entry read failed");
            assert_eq!(
                entry.compression(),
                zip::CompressionMethod::Stored,
                "File {} in .multisample was not stored uncompressed!",
                entry.name()
            );
        }

        let midi_test_path = temp_dir.join("test_export.mid");
        assert!(midi_test_path.exists(), "Associated .mid file was not created alongside .multisample!");

        let _ = std::fs::remove_file(test_path);
        let _ = std::fs::remove_file(midi_test_path);
    }
}
