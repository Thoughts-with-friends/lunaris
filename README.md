# Lunaris

<div align="center">
  <a href="https://github.com/Thoughts-with-friends/lunaris/releases">
    <img src="./docs/icons/icon.svg" alt="Lunaris"/>
  </a>

  <!-- Release Badges -->
  <p>
    <a href="https://github.com/Thoughts-with-friends/lunaris/releases/latest">
      <img src="https://img.shields.io/github/v/release/Thoughts-with-friends/lunaris?style=flat-square" alt="Latest Release">
    </a>
    <a href="https://github.com/Thoughts-with-friends/lunaris/releases">
      <img src="https://img.shields.io/github/downloads/Thoughts-with-friends/lunaris/total?style=flat-square" alt="Total Downloads">
    </a>
    <!-- <a href="https://github.com/Thoughts-with-friends/lunaris/actions/workflows/release-gui.yaml">
      <img src="https://github.com/Thoughts-with-friends/lunaris/actions/workflows/release-gui.yaml/badge.svg?style=flat-square" alt="Release GUI Status">
    </a> -->
    <a href="https://opensource.org/licenses/GPL-3.0">
      <img src="https://img.shields.io/badge/License-GPLv3-blue.svg?style=flat-square" alt="License: GPL v3">
    </a>
    <a href="https://github.com/Thoughts-with-friends/lunaris/stargazers">
      <img src="https://img.shields.io/github/stars/Thoughts-with-friends/lunaris?style=social" alt="GitHub Stars">
    </a>
  </p>

  <!-- Development Badges -->
  <p>
    <a href="https://github.com/Thoughts-with-friends/lunaris/actions/workflows/build-emu.yaml">
      <img src="https://github.com/Thoughts-with-friends/lunaris/actions/workflows/build-emu.yaml/badge.svg?style=flat-square" alt="Build & Test Status">
    </a>
    <a href="https://github.com/Thoughts-with-friends/lunaris/issues">
      <img src="https://img.shields.io/github/issues/Thoughts-with-friends/lunaris?style=flat-square" alt="Open Issues">
    </a>
    <a href="https://github.com/Thoughts-with-friends/lunaris/pulls">
      <img src="https://img.shields.io/github/issues-pr/Thoughts-with-friends/lunaris?style=flat-square" alt="Open PRs">
    </a>
    <a href="https://github.com/Thoughts-with-friends/lunaris/commits/main">
      <img src="https://img.shields.io/github/last-commit/Thoughts-with-friends/lunaris?style=flat-square" alt="Last Commit">
    </a>
    <a href="https://github.com/Thoughts-with-friends/lunaris/graphs/contributors">
      <img src="https://img.shields.io/github/contributors/Thoughts-with-friends/lunaris?style=flat-square" alt="Contributors">
    </a>
    <a href="https://github.com/Thoughts-with-friends/lunaris">
      <img src="https://img.shields.io/github/languages/top/Thoughts-with-friends/lunaris?style=flat-square" alt="Top Language">
    </a>
    <a href="https://github.com/Thoughts-with-friends/lunaris">
      <img src="https://img.shields.io/github/languages/code-size/Thoughts-with-friends/lunaris?style=flat-square" alt="Code Size">
    </a>
  </p>
</div>

A Nintendo DS emulator - Rust-based

## Quick Start

- Imgui

```shell
cargo run --release --features release --package lunaris
```

- Egui

```shell
cargo build --release  # default or
cargo run --release --features release --package lunaris-egui
```

## Todo List

- [x] **Phase 0 (100%)**: FreeBIOS Development
- [ ] **Phase 1 (0%)**: Optimization / Debugging
  - [x] **Phase 1-1 (100%)**: Support Controller Input
  - [ ] **Phase 1-2 (0%)**: Save Data System (.sav)
  - [ ] **Phase 1-3 (0%)**: 2D Graphics
  - [ ] **Phase 1-4 (0%)**: 3D Graphics
  - [ ] **Phase 1-5 (0%)**: Loading Any ROM
- [ ] **Phase 2 (0%)**: WiFi / Networking
- [ ] **Phase 3 (0%)**: VPN Network support
- [ ] **Phase 4 (0%)**: JIT Compile


## CI Tests

Our CI tests are run using [GitHub Actions](https://github.com/Thoughts-with-friends/lunaris/actions).

## References

This project is a modified/extended version based on NDS-Emulator.

- [NDS Emulator](https://github.com/Ace314159/NDS-Emulator/tree/e7c8a317db7e1d370f90a17637b338782737b528): Base on the original NDS Emulator
- [dust](https://github.com/kelpsyberry/dust): A Nintendo DS emulator written in Rust
- [CorgiDS](https://github.com/PSI-Rockin/CorgiDS): A dog-themed DS emulator
- [desmume](https://github.com/TASEmulators/desmume): Nintendo DS emulator written in C and C++
- [melonDS](https://github.com/melonDS-emu/melonDS): DS emulator high performance, low memory usage
- [GBATEK](https://problemkaputt.de/gbatek.htm): GBA / Nintendo DS / DSi / 3DS - Technical Info
- [TinyFB.nds](https://imrannazar.com/The-Smallest-NDS-File): The smallest Nintendo DS file
