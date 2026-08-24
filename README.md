# SlicePlayer 🎹⚡

**SlicePlayer** is an open-source, high-performance **CLAP audio plugin** for Linux designed for dynamic audio loop slicing, real-time playback, and direct export to **Bitwig Studio `.multisample` archives** with synchronized MIDI pattern clips.

Built with Rust, [`nih-plug`](https://github.com/robbert-vdh/nih-plug), [`egui`](https://github.com/emilk/egui), and C++ REX2/DWOP codec support via **VelociLoops**.

---

## ✨ Features

- **🎧 Audio Slicing & Onset Detection**:
  - Transient detection using SuperFlux spectral onset analysis.
  - Beat grid slicing (1/4, 1/8, 1/16, 1/32, triplets).
  - Manual slice creation, boundary adjustment, and slice editing.

- **📦 Bitwig Studio `.multisample` Export**:
  - Exports uncompressed (STORED ZIP method) `.multisample` archives streaming-ready for Bitwig Studio Sampler.
  - Native 1:1 Bitwig XML schema generation (`<key high="..." low="..." root="..." track="0.0000" tune="0.00"/>`).
  - Automatic zero-pitch tracking (`track="0.0000"`) ensuring original transient playback speed across all keys.

- **🎹 Companion MIDI Pattern Export**:
  - Automatically exports a companion `.mid` clip alongside the `.multisample` archive with identical stem filename.
  - One-click **"Copy MIDI"** button to copy `text/uri-list` straight to your system clipboard for instant drag-and-drop / Ctrl+V into Bitwig or REAPER.

- **⚡ Real-Time Audio Engine**:
  - Zero-heap allocation audio rendering loop for real-time safety.
  - Per-slice volume gain, pitch shift, reverse playback, and fade envelopes.
  - REX / REX2 (`.rx2`, `.rex`, `.rcy`) import support.

---

## 🚀 Quick Start & Installation

### Pre-built Linux Binary

The latest pre-built Linux CLAP plugin is available directly in the repository under [`bin/slice_player.clap`](bin/slice_player.clap).

Copy it to your local CLAP directory:

```bash
mkdir -p ~/.clap
cp bin/slice_player.clap ~/.clap/
```

### Building from Source

#### Prerequisites
- Rust 1.75+ toolchain
- C++17 compatible compiler (`g++` or `clang++`)
- `cmake` (for C++ dependencies)

#### Build Commands

```bash
# Clone the repository
git clone --recursive https://github.com/kiklabautermann/sliceplayer.git
cd sliceplayer

# Build release bundle
cargo run --package xtask -- bundle

# Installed CLAP binary will be located at:
# target/release/libslice_player.so
```

---

## 🛠️ Tech Stack & Architecture

- **Audio Plugin Framework**: [`nih-plug`](https://github.com/robbert-vdh/nih-plug) (CLAP standard)
- **GUI Framework**: [`egui`](https://github.com/emilk/egui) / `nih_plug_egui`
- **Audio Codecs**: REX2 DWOP decoding via VelociLoops FFI & `symphonia` for WAV/FLAC/MP3 decoding
- **MIDI Serialization**: [`midly`](https://github.com/Bavards/midly)
- **ZIP Streamer**: [`zip-rs`](https://github.com/zip-rs/zip)

---

## 🙏 Credits & Acknowledgements

SlicePlayer relies on incredible open-source libraries:

- **[VelociLoops](https://github.com/kunitoki/VelociLoops)** by [@kunitoki](https://github.com/kunitoki): A clean-room C library for reading, writing, and transient-slicing REX2 (`.rx2`), REX (`.rex`), and RCY (`.rcy`) audio containers and decoding the DWOP bitstream.
- **[nih-plug](https://github.com/robbert-vdh/nih-plug)** by [@robbert-vdh](https://github.com/robbert-vdh): Rust audio plugin framework for CLAP and VST3.

---

## 📄 License

This project is open-source under the MIT License. See [LICENSE](LICENSE) for details.
