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

```shell
cargo build --release
```

## Todo List

- [x] **Phase 0**: FreeBIOS Development
- [ ] **Phase 1**: Foundation Setup (Memory, Constants)
- [ ] **Phase 2**: CPU Core Implementation
- [ ] **Phase 3**: Memory / I/O Management
- [ ] **Phase 4**: GPU Infrastructure
- [ ] **Phase 5**: BIOS / ROM Loading
- [ ] **Phase 6**: UI / Threading
- [ ] **Phase 7**: Audio System
- [ ] **Phase 8**: Interrupt System
- [ ] **Phase 9**: Instruction Set Completion (ARM9)
- [ ] **Phase 10**: ARM7 Implementation
- [ ] **Phase 11**: 3D Graphics
- [ ] **Phase 12**: Save Data System
- [ ] **Phase 13**: WiFi / Networking
- [ ] **Phase 14**: Optimization / Debugging

## References

- [CorgiDS](https://github.com/PSI-Rockin/CorgiDS): A dog-themed DS emulator
- [melonDS](https://github.com/melonDS-emu/melonDS): DS emulator, sorta
- [dust](https://github.com/kelpsyberry/dust): A Nintendo DS emulator written in Rust
- [GBATEK](https://problemkaputt.de/gbatek.htm): GBA / Nintendo DS / DSi / 3DS - Technical Info
