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

## How to Build

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

## Todo List

- [x] **Phase 0 (100%)**: FreeBIOS Development
- [x] **Phase 1 (100%)**: Foundation Setup (Memory, Constants)
- [x] **Phase 2 (100%)**: CPU Core Implementation
- [x] **Phase 3 (100%)**: Memory / I/O Management
- [x] **Phase 4 (100%)**: GPU Infrastructure
- [x] **Phase 5 (100%)**: BIOS / ROM Loading
- [ ] **Phase 6 ( 70%)**: UI / Threading
- [ ] **Phase 7 ( 40%)**: Audio System
- [x] **Phase 8 (100%)**: Interrupt System
- [x] **Phase 9 (100%)**: Instruction Set Completion (ARM9)
- [x] **Phase 10 (100%)**: ARM7 Implementation
- [x] **Phase 11 (100%)**: 3D Graphics
- [ ] **Phase 12 ( 20%)**: Save Data System
- [ ] **Phase 13 ( 80%)**: WiFi / Networking
- [ ] **Phase 14 ( 20%)**: Optimization / Debugging
- [ ] **Phase 15 ( 0%)**: VPN Network support
- [ ] **Phase 16 ( 0%)**: JIT Compile

## Dependencies

- rust stable = "1.91"
- Backend: ./core
  - tracing = "0.1.44"
  - snafu = "0.8.9"

- Frontend: ./gui/tauri
  - tauri-build = 2.5.3
  - [Other Web Libraries](gui/tauri/package.json)

## CI Tests

- [Lunaris CI](https://github.com/Thoughts-with-friends/lunaris/actions)

## References

- [CorgiDS](https://github.com/PSI-Rockin/CorgiDS): A dog-themed DS emulator
- [dust](https://github.com/kelpsyberry/dust): A Nintendo DS emulator written in Rust
- [desmume](https://github.com/TASEmulators/desmume): Nintendo DS emulator written in C and C++
- [melonDS](https://github.com/melonDS-emu/melonDS): DS emulator, sorta
- [GBATEK](https://problemkaputt.de/gbatek.htm): GBA / Nintendo DS / DSi / 3DS - Technical Info
