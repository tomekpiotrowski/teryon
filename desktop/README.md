# Teryon's desktop app

A window around the Angular app in `app/`, which starts the local `teryon`
service when it is not running. The service ships inside every package, next
to the app.

## Installing

Each release on GitHub has the Linux packages, and `SHA256SUMS` to check them:

- **AppImage**: runs on any recent Linux. `chmod +x Teryon_*.AppImage` and start it.
- **.deb** for Debian and Ubuntu: `sudo apt install ./Teryon_*_amd64.deb`
- **.rpm** for Fedora: `sudo dnf install ./Teryon-*.x86_64.rpm`
- **Arch and Omarchy**: `makepkg -si` in `arch/`, which downloads the .deb from the release.

Teryon runs the agents you already have installed (`claude`, `codex`, `grok`)
and needs `git`.

There are no automatic updates yet: install a newer package over the old one.
A service of the old version keeps running until the app offers to restart it.

## Building a release

1. Change the version in `Cargo.toml`, `app/package.json`, `www/package.json`
   and `arch/PKGBUILD`, and commit.
2. Tag the commit on main and push the tag: `git tag v0.2.0 && git push origin v0.2.0`.
3. The Release workflow builds the packages into a draft release. Try them,
   then publish the draft.
4. Run `updpkgsums` in `arch/` to record the new .deb's checksum, and commit.

To build the packages on your own machine: `cargo tauri build` in `src-tauri/`.
They land in `target/release/bundle/`. On Arch and Omarchy the AppImage step
fails unless stripping is turned off, because the `strip` inside linuxdeploy is
too old for Arch's libraries: `NO_STRIP=true cargo tauri build`.
