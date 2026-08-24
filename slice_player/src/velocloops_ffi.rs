//! FFI bindings to VelociLoops (C API).
#![allow(non_camel_case_types, dead_code)]

use std::ffi::{CStr, CString};
use std::path::Path;

// ── Opaque handle ────────────────────────────────────────────────────────────
#[repr(C)]
struct VLFile_s {
    _opaque: [u8; 0],
}
type VLFile = *mut VLFile_s;

// ── Error enum ───────────────────────────────────────────────────────────────
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VLError {
    Ok = 0,
    InvalidHandle = -1,
    InvalidArg = -2,
    FileNotFound = -3,
    FileCorrupt = -4,
    OutOfMemory = -5,
    InvalidSlice = -6,
    InvalidSampleRate = -7,
    BufferTooSmall = -8,
    NoCreatorInfo = -9,
    NotImplemented = -10,
    AlreadyHasData = -11,
    FileTooNew = -12,
    ZeroLoopLength = -13,
    InvalidSize = -14,
    InvalidTempo = -15,
}

impl VLError {
    pub fn is_ok(self) -> bool { self == VLError::Ok }
}

// ── Structs ──────────────────────────────────────────────────────────────────
#[repr(C)]
#[derive(Debug, Default, Clone)]
pub struct VLFileInfo {
    pub channels: i32,
    pub sample_rate: i32,
    pub slice_count: i32,
    pub tempo: i32,
    pub original_tempo: i32,
    pub ppq_length: i32,
    pub time_sig_num: i32,
    pub time_sig_den: i32,
    pub bit_depth: i32,
    pub total_frames: i32,
    pub loop_start: i32,
    pub loop_end: i32,
    pub processing_gain: i32,
    pub transient_enabled: i32,
    pub transient_attack: i32,
    pub transient_decay: i32,
    pub transient_stretch: i32,
    pub silence_selected: i32,
}

#[repr(C)]
#[derive(Debug, Default, Clone)]
pub struct VLSliceInfo {
    pub ppq_pos: i32,
    pub sample_length: i32,
    pub sample_start: i32,
    pub analysis_points: i32,
    pub flags: i32,
}

/// SuperFlux onset detection options (mirrors VLSuperFluxOptions).
#[repr(C)]
#[derive(Debug, Clone)]
pub struct VLSuperFluxOptions {
    pub frame_size: i32,
    pub fps: i32,
    pub filter_bands: i32,
    pub max_bins: i32,
    pub diff_frames: i32,
    pub min_slice_frames: i32,
    pub filter_equal: i32,
    pub online: i32,
    pub threshold: f32,
    pub combine_ms: f32,
    pub pre_avg: f32,
    pub pre_max: f32,
    pub post_avg: f32,
    pub post_max: f32,
    pub delay_ms: f32,
    pub ratio: f32,
    pub fmin: f32,
    pub fmax: f32,
    pub log_mul: f32,
    pub log_add: f32,
}

impl Default for VLSuperFluxOptions {
    fn default() -> Self {
        // Mirror library defaults from header.
        Self {
            frame_size: 2048,
            fps: 200,
            filter_bands: 24,
            max_bins: 3,
            diff_frames: 0,
            min_slice_frames: 0,
            filter_equal: 0,
            online: 0,
            threshold: 1.1,
            combine_ms: 50.0,
            pre_avg: 0.15,
            pre_max: 0.01,
            post_avg: 0.0,
            post_max: 0.05,
            delay_ms: 0.0,
            ratio: 0.5,
            fmin: 30.0,
            fmax: 17000.0,
            log_mul: 1.0,
            log_add: 1.0,
        }
    }
}

// ── extern "C" declarations ──────────────────────────────────────────────────
unsafe extern "C" {
    fn vl_open(path: *const i8, err: *mut i32) -> VLFile;
    fn vl_close(file: VLFile);
    fn vl_get_info(file: VLFile, out: *mut VLFileInfo) -> i32;
    fn vl_get_slice_info(file: VLFile, index: i32, out: *mut VLSliceInfo) -> i32;
    fn vl_get_slice_frame_count(file: VLFile, index: i32) -> i32;
    fn vl_decode_slice(
        file: VLFile, index: i32,
        left: *mut f32, right: *mut f32,
        frame_offset: i32, capacity: i32, frames_out: *mut i32,
    ) -> i32;
    fn vl_superflux_default_options(out: *mut VLSuperFluxOptions);
    fn vl_create_from_superflux(
        channels: i32, sample_rate: i32, tempo: i32,
        left: *const f32, right: *const f32, frames: i32,
        options: *const VLSuperFluxOptions, err: *mut i32,
    ) -> VLFile;
    fn vl_error_string(err: i32) -> *const i8;
    fn vl_version_string() -> *const i8;
}

// ── Safe Rust wrapper ─────────────────────────────────────────────────────────

/// RAII wrapper around an open VLFile handle.
pub struct Rex2File {
    handle: VLFile,
    pub info: VLFileInfo,
}

unsafe impl Send for Rex2File {}
unsafe impl Sync for Rex2File {}

impl Rex2File {
    /// Open a .rx2 file from disk.
    pub fn open(path: &Path) -> Result<Self, String> {
        let c = CString::new(path.to_str().ok_or("bad path")?).map_err(|e| e.to_string())?;
        let mut err: i32 = 0;
        let handle = unsafe { vl_open(c.as_ptr(), &mut err) };
        if handle.is_null() {
            let msg = unsafe { CStr::from_ptr(vl_error_string(err)) };
            return Err(msg.to_string_lossy().into_owned());
        }
        let mut info = VLFileInfo::default();
        unsafe { vl_get_info(handle, &mut info) };
        Ok(Self { handle, info })
    }

    pub fn slice_count(&self) -> i32 { self.info.slice_count }

    /// BPM as f64 (stored as BPM×1000 in the file).
    pub fn bpm(&self) -> f64 { self.info.tempo as f64 / 1000.0 }

    pub fn get_slice_info(&self, index: i32) -> Option<VLSliceInfo> {
        let mut si = VLSliceInfo::default();
        let r = unsafe { vl_get_slice_info(self.handle, index, &mut si) };
        if r == 0 { Some(si) } else { None }
    }

    /// Decode slice `index` into interleaved stereo `Vec<f32>`.
    pub fn decode_slice_stereo(&self, index: i32) -> Result<Vec<f32>, String> {
        let n = unsafe { vl_get_slice_frame_count(self.handle, index) };
        if n <= 0 { return Err(format!("invalid slice {index}")); }
        let mut left = vec![0f32; n as usize];
        let mut right = vec![0f32; n as usize];
        let mut written: i32 = 0;
        let r = unsafe {
            vl_decode_slice(self.handle, index,
                left.as_mut_ptr(), right.as_mut_ptr(),
                0, n, &mut written)
        };
        if r != 0 {
            let msg = unsafe { CStr::from_ptr(vl_error_string(r)) };
            return Err(msg.to_string_lossy().into_owned());
        }
        // Interleave L/R → stereo
        let mut out = Vec::with_capacity(written as usize * 2);
        for i in 0..written as usize {
            out.push(left[i]);
            out.push(right[i]);
        }
        Ok(out)
    }

    pub fn vl_version() -> &'static str {
        unsafe { CStr::from_ptr(vl_version_string()).to_str().unwrap_or("?") }
    }
}

impl Drop for Rex2File {
    fn drop(&mut self) {
        unsafe { vl_close(self.handle); }
    }
}

/// Run VelociLoops SuperFlux onset detection on raw PCM and return detected
/// slice positions in sample frames. Used by our Transient-Detection mode.
pub fn superflux_detect_slices(
    left: &[f32],
    right: Option<&[f32]>,
    sample_rate: u32,
    bpm: f64,
    options: &VLSuperFluxOptions,
) -> Result<Vec<i32>, String> {
    let channels = if right.is_some() { 2 } else { 1 };
    let frames = left.len() as i32;
    let tempo = (bpm * 1000.0) as i32;
    let right_ptr = right.map(|r| r.as_ptr()).unwrap_or(std::ptr::null());

    let mut err: i32 = 0;
    let handle = unsafe {
        vl_create_from_superflux(
            channels, sample_rate as i32, tempo,
            left.as_ptr(), right_ptr, frames,
            options as *const _, &mut err,
        )
    };

    if handle.is_null() {
        let msg = unsafe { CStr::from_ptr(vl_error_string(err)) };
        return Err(msg.to_string_lossy().into_owned());
    }

    let mut info = VLFileInfo::default();
    unsafe { vl_get_info(handle, &mut info) };

    let mut positions = Vec::new();
    for i in 0..info.slice_count {
        let mut si = VLSliceInfo::default();
        unsafe { vl_get_slice_info(handle, i, &mut si) };
        positions.push(si.sample_start);
    }

    unsafe { vl_close(handle) };
    Ok(positions)
}

/// Populate a VLSuperFluxOptions with library defaults.
pub fn superflux_default_options() -> VLSuperFluxOptions {
    let mut opts = VLSuperFluxOptions::default();
    unsafe { vl_superflux_default_options(&mut opts) };
    opts
}
