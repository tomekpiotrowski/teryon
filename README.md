# Teryon

Teryon runs coding agents such as Claude Code, Codex and Grok on your projects.
Each piece of work gets its own copy of the code, so several agents can work
at once without getting in each other's way, and they keep working after the
window closes. You read along, answer their questions, and apply the changes
you like.

## Installing

Version 0.1 is a desktop app for Linux. Download it from the
[releases](https://github.com/tomekpiotrowski/teryon/releases):

- **AppImage**: runs on any recent Linux. `chmod +x Teryon_*.AppImage` and start it.
- **.deb** for Debian and Ubuntu: `sudo apt install ./Teryon_*_amd64.deb`
- **.rpm** for Fedora: `sudo dnf install ./Teryon-*.x86_64.rpm`
- **Arch and Omarchy**: `makepkg -si` in [`desktop/arch`](desktop/arch).

Teryon needs `git`, and runs the agents you already have installed and signed in to.

## Building

The parts:

- `core/`: the library that runs agents and manages the copies of the code.
- `service/`: `teryon`, the local service the app talks to, and the command agents use.
- `app/`: the interface, in Angular.
- `desktop/`: the desktop app around it, in Tauri.
- `api/`, `admin/`, `www/`: the server for groups, its admin screens, and the website. Not part of 0.1.

You need Rust, [bun](https://bun.sh), and the
[libraries Tauri needs](https://tauri.app/start/prerequisites/). Then:

```sh
cd app && bun install && cd ..
cargo test                         # the core library and the service
cd desktop/src-tauri && cargo tauri dev
```

[`desktop/README.md`](desktop/README.md) covers building the packages and making a release.

## Licence

[MIT](LICENSE)
