# CogiDS-rs

<div align="center">
  <a href="https://github.com/Thoughts-with-friends/cogids-rs/releases">
    <img src="./docs/icons/icon.svg" alt="Lunaris"/>
  </a>

  <!-- Release Badges -->
  <p>
    <a href="https://github.com/Thoughts-with-friends/cogids-rs/releases/latest">
      <img src="https://img.shields.io/github/v/release/Thoughts-with-friends/lunaris?style=flat-square" alt="Latest Release">
    </a>
    <a href="https://github.com/Thoughts-with-friends/cogids-rs/releases">
      <img src="https://img.shields.io/github/downloads/Thoughts-with-friends/lunaris/total?style=flat-square" alt="Total Downloads">
    </a>
    <!-- <a href="https://github.com/Thoughts-with-friends/cogids-rs/actions/workflows/release-gui.yaml">
      <img src="https://github.com/Thoughts-with-friends/cogids-rs/actions/workflows/release-gui.yaml/badge.svg?style=flat-square" alt="Release GUI Status">
    </a> -->
    <a href="https://opensource.org/licenses/GPL-3.0">
      <img src="https://img.shields.io/badge/License-GPLv3-blue.svg?style=flat-square" alt="License: GPL v3">
    </a>
    <a href="https://github.com/Thoughts-with-friends/cogids-rs/stargazers">
      <img src="https://img.shields.io/github/stars/Thoughts-with-friends/lunaris?style=social" alt="GitHub Stars">
    </a>
  </p>

  <!-- Development Badges -->
  <p>
    <a href="https://github.com/Thoughts-with-friends/cogids-rs/actions/workflows/build-emu.yaml">
      <img src="https://github.com/Thoughts-with-friends/cogids-rs/actions/workflows/build-emu.yaml/badge.svg?style=flat-square" alt="Build & Test Status">
    </a>
    <a href="https://github.com/Thoughts-with-friends/cogids-rs/issues">
      <img src="https://img.shields.io/github/issues/Thoughts-with-friends/lunaris?style=flat-square" alt="Open Issues">
    </a>
    <a href="https://github.com/Thoughts-with-friends/cogids-rs/pulls">
      <img src="https://img.shields.io/github/issues-pr/Thoughts-with-friends/lunaris?style=flat-square" alt="Open PRs">
    </a>
    <a href="https://github.com/Thoughts-with-friends/cogids-rs/commits/main">
      <img src="https://img.shields.io/github/last-commit/Thoughts-with-friends/lunaris?style=flat-square" alt="Last Commit">
    </a>
    <a href="https://github.com/Thoughts-with-friends/cogids-rs/graphs/contributors">
      <img src="https://img.shields.io/github/contributors/Thoughts-with-friends/lunaris?style=flat-square" alt="Contributors">
    </a>
    <a href="https://github.com/Thoughts-with-friends/cogids-rs">
      <img src="https://img.shields.io/github/languages/top/Thoughts-with-friends/lunaris?style=flat-square" alt="Top Language">
    </a>
    <a href="https://github.com/Thoughts-with-friends/cogids-rs">
      <img src="https://img.shields.io/github/languages/code-size/Thoughts-with-friends/lunaris?style=flat-square" alt="Code Size">
    </a>
  </p>
</div>

A Nintendo DS emulator - Rust-based

## How to Build

### for Tauri

- Install nvm: Example - v0.39.7 on Linux.

```shell
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.7/install.sh | bash
nvm install --lts
npm -v  # e.g. 11.10.0
```

- Build

```shell
npm i
npm run build
```

### egui

```bash
# build
cargo build -p lunaris_ds_egui --release

# build and run
cargo run -p lunaris_ds_egui --release
```

### freya

Borderless, translucent window ([freya](https://github.com/marc2332/freya) 0.5, needs Rust 1.99+).

```bash
cargo run -p lunaris_ds_freya --release

# headless snapshots of every screen into <temp>/lunaris_freya_snapshots
# (the game ones need the ROMs under D:/GAME/Emulator/DS/Roms)
cargo test --release -p lunaris_ds_freya -- --ignored
```

## Todo List

- [x] **Phase 0 (100%)**: FreeBIOS Development
- [x] **Phase 1 (100%)**: Foundation Setup (Memory, Constants)
- [x] **Phase 2 (100%)**: CPU Core Implementation
- [x] **Phase 3 (100%)**: Memory / I/O Management
- [x] **Phase 4 (100%)**: GPU Infrastructure
- [x] **Phase 5 (100%)**: BIOS / ROM Loading
- [x] **Phase 6 (100%)**: UI / Threading
- [x] **Phase 7 (100%)**: Audio System
- [x] **Phase 8 (100%)**: Interrupt System
- [x] **Phase 9 (100%)**: Instruction Set Completion (ARM9)
- [x] **Phase 10 (100%)**: ARM7 Implementation
- [x] **Phase 11 (100%)**: 3D Graphics
- [x] **Phase 12 (100%)**: Save Data System
- [x] **Phase 13 (100%)**: WiFi / Networking
- [ ] **Phase 14 ( 20%)**: Optimization / Debugging
- [ ] **Phase 15 ( 0%)**: VPN Network support
- [ ] **Phase 16 ( 0%)**: JIT Compile

## Dependencies

- rust stable = "1.91"
- Backend: ./core
  - log = "0.1.44"
  - snafu = "0.8.9"

- Frontend: ./gui/tauri
  - tauri-build = 2.5.3
  - [Other Web Libraries](gui/tauri/package.json)

## CI Tests

- [Lunaris CI](https://github.com/Thoughts-with-friends/cogids-rs/actions)

## References

This project is a modified/extended version based on NDS-Emulator.

- Emulators
  - [NDS Emulator](https://github.com/Ace314159/NDS-Emulator/tree/e7c8a317db7e1d370f90a17637b338782737b528): NDS Emulator written in Rust
  - [dust](https://github.com/kelpsyberry/dust): A Nintendo DS emulator written in Rust
  - [CorgiDS](https://github.com/PSI-Rockin/CorgiDS): A dog-themed DS emulator (Base on this)
  - [desmume](https://github.com/TASEmulators/desmume): Nintendo DS emulator written in C and C++
  - [melonDS](https://github.com/melonDS-emu/melonDS): DS emulator high performance, low memory usage

- Tools
  - [melonds-rs](https://github.com/tangobattle/melonds-rs): Wrapper for the melonDS library
  - [melonds-rs (folk ver.)](https://github.com/Thoughts-with-friends/melonds-rs): Wrapper for the melonDS library

- Technical Information
  - [Nintendo DS Architecture](https://www.copetti.org/writings/consoles/nintendo-ds/): An overview of the NDS
  - [GBATEK](https://problemkaputt.de/gbatek.htm): GBA / Nintendo DS / DSi / 3DS - Technical Info
  - [GBATEK-md](https://mgba-emu.github.io/gbatek/#dstechnicaldata): GBA / Nintendo DS / DSi / 3DS - Technical Info
  - [BlocksDS](https://blocksds.skylyrac.net/tutorial/): Details on the NDS, focusing on easy-to-understand specifications at the implementation stage
  - [TinyFB.nds](https://imrannazar.com/The-Smallest-NDS-File): The smallest Nintendo DS file

