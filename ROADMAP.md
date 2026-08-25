# SlicePlayer Roadmap & Feature Ideas 🗺️⚡

This document outlines planned features, community ideas, and future enhancements for **SlicePlayer**.

---

## 🎯 Planned Features & Enhancements

### 📦 DAW Export Expansion
- [x] **Bitwig Multisample Export**: Native `.multisample` export for Bitwig Studio.
- [x] **Bitwig MIDI Clip Copy**: One-click `🎹 Copy MIDI` to clipboard (`text/uri-list`) for instant `Ctrl+V` pasting into Bitwig Studio with exact bar length and grid clamping.
- [ ] **Custom WAV Export Scope**: Option/toggle to export slice WAV files either completely or restricted strictly to the active loop/slice region.
- [ ] **Ableton Live Drum Rack Export**: Export `.alc` / Drum Rack presets.
- [ ] **DecentSampler Preset Export**: Export `.dspreset` XML archives.
- [ ] **FL Studio Slicex / DirectWave Export**: Export native preset formats.
- [ ] **SFZ Format Export**: Export generic `.sfz` instrument mappings for Linux/macOS sampler plugins.

### 🎧 Audio & DSP Engine
- [x] **Per-Slice Choke Groups**: Allow slices (e.g., Open/Closed Hi-Hats) to cut each other off during playback.
- [x] **Per-Slice Filter & Processing**: Resonant Lowpass & Highpass filters per slice (DJM Mixer LP/HP Combo with resonance).
- [x] **Per-Slice Bitcrusher & Degradation**: Bit depth reduction (2–16 bits), sample rate crushing (downsampling 1–16x), and soft drive distortion per slice.
- [x] **Per-Slice Retrigger / Rhythmic Stutter**: Multi-trigger repeat modes (1/8, 1/16, 1/32, 1/64 rolls) with decay envelope per slice.
- [x] **Akai-Style Vintage Time-Stretching**: Classic S1000/S3000 granular pitch-coarse time-stretching (0.5x–2.0x) with signature metallic artifacts.
- [x] **DJM500 / Oldschool Jungle Dub Echo**: Rhythmic BPM-synced delay (1/64 to 1/2, 1/16 dotted, 1ms–100ms free ms) with feedback LP filtering for classic D&B / Jungle dub echoes.
- [ ] **Velocity Sensitivity Mapping**: Customizable velocity curves per slice.

### 🌴 Jungle & Drum & Bass Specialized Tools
- [x] **Per-Slice Reverse Toggle**: Instant reverse playback per slice for swell effects & reverse snare fills.
- [x] **Zero-Crossing Auto-Snap & Micro-Nudge**: Zero-crossing alignment on marker drag/creation AND keyboard arrow key (`←` / `→`) nudge mode when hovering slice markers.
- [ ] **Multi-Bus Audio Routing (Multi-Out)**: Route slices to up to 8 separate stereo DAW audio buses (e.g., Bus 1: Kick, Bus 2: Snare, Bus 3: Tops, Bus 4: FX).
- [ ] **Per-Slice Pitch Envelope & Pitch Drop**: Pitch decay envelope per slice for classic 808/snare pitch-drops.
- [x] **Jungle Ghost Note & Break Shuffler**: 1-click generative syncopation & snare roll generator with 4 algorithmic styles (Amen Roller, Syncopated Funk, Ghost Notes Only, Wild Chopper) and beat locking.
- [ ] **E-mu Z-Plane / Akai S950 Filter Modeling**: Non-linear warm lowpass filter modeling with sampler saturation.

### 🎨 GUI & User Experience
- [x] **MIDI Triggered Slice Visual Highlighting**: Real-time 60 FPS neon cyan glow, glowing border, and `▶ PLAY #XX` badge overlay on slices currently played via MIDI or GUI triggers.
- [x] **Multi-Slice Selection (Ctrl+Click & Ctrl+A)**: Select multiple slices with Ctrl+Click or select ALL slices simultaneously with Ctrl+A / Batch Action button, modifying parameters (Gain, Pan, Pitch, Filters, FX, Mute, Reverse) for all selected slices in real time.
- [x] **Copy & Paste Slice Settings**: Copy all FX, filter, envelope, and pitch settings from one slice and apply/propagate them to all slices simultaneously.
- [x] **Reset Slice Settings to Default**: Reset per-slice FX and tuning parameters back to default initial state per slice.
- [x] **Mouse Wheel Zoom & Pan**: Pinch-to-zoom, middle-mouse drag panning, horizontal scroll wheel panning, and multi-resolution sample-accurate waveform rendering.
- [ ] **Direct Waveform Drag-and-Drop**: Drag audio files directly from the OS file manager into the waveform view.
- [ ] **Slice Categorization & Color Tagging**: Assign custom colors and tags (Kick, Snare, HiHat, Synth, Perc) to slices.
- [ ] **A/B Comparison Snapshots**: Quick state snapshot switching while editing slice layouts.

### ⚙️ Settings & State Management
- [x] **Full Preset Save & Load**: Save and restore complete SlicePlayer presets (`.slicepreset`) including all per-slice FX settings, filters, delays, slice points, and mapping configurations.
- [x] **Plugin Settings & Favorite Paths (JSON/XML)**: Load & save global plugin configuration files (`global_settings.json`) for favorite directory paths, default export folders, and custom UI preferences.

---

## 💬 Feature Requests & Community Feedback

Have a feature request or idea?
Feel free to open an issue or submit suggestions on [GitHub Issues](https://github.com/kiklabautermann/sliceplayer/issues)!
