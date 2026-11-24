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

Teryon runs the agents you already have installed (`claude`, `codex`, `grok`, `agy`)
and needs `git`.

There are no automatic updates yet: install a newer package over the old one.
A service of the old version keeps running until the app offers to restart it.

## The terminal command in an AppImage

Opening an AppImage starts background setup of `teryon` for the current user,
without administrator access or delaying the window. The command lives in
`$XDG_DATA_HOME/teryon/commands/<version>/teryon` (normally
`~/.local/share/teryon/commands/<version>/teryon`), with a link at
`~/.local/bin/teryon`. It works after the AppImage closes. Opening a newer
release updates the link; opening an older one does not downgrade it.
A matching AppImage repairs its damaged copy. If a newer copy is missing or
damaged, an older AppImage offers an explicit **Use this AppImage’s version**
recovery action; it never silently downgrades.

Old versioned copies are retained so running services can still launch agents
through their executable paths. Automatic deletion is deferred until services
can hold a lifetime lease on their copy; a process scan alone races with new
service launches. Disk use therefore grows with installed releases. After
stopping all Teryon services, unused version directories can be removed
manually; keep the version selected by `readlink ~/.local/bin/teryon`.

If `~/.local/bin/teryon` already exists outside this managed installation,
Teryon leaves it alone and offers **Replace existing command**. This also
applies to a command previously copied out of an AppImage by hand. Replacing
a link leaves its original target alone. A directory must be moved manually.

If the command is missing from the shell's search path, or another `teryon`
takes precedence, the app shows setup instructions for Bash, Zsh, or fish.
It never changes shell configuration automatically. The check uses the
app's environment; after changing the search path, open a new terminal and
start Teryon from that terminal. `~/.local/bin/teryon --help` also works directly.

Setup failures leave the desktop app usable. A dedicated thread runs the
work, so the window opens while setup is still going. A version check gives
up after two seconds and the stuck check is stopped. A second launch waits
for one install to finish copying the command, saving it, and running those
checks, then reports that result. It reports a busy installation only if
that wait runs out. The notice arrives when the worker finishes. **Retry
installation** is offered for installation problems. A problem saving the
setup record is shown separately and does not, by itself, offer a retry.
Search-path guidance instead says to restart the app: an existing process
cannot inherit a new shell's environment. **Dismiss** hides the notice until
the next launch.

Failures identify the operation and path. The latest completed result is
saved atomically to `terminal-command.json` in the service's logs directory:
normally `~/.local/state/teryon/logs`, or `$TERYON_HOME/logs` when overridden.
The record is replaced on completion instead of growing indefinitely.
Installing the command does not stop agents or restart the service.

The installer and controller tests run with `cargo test -p teryon-desktop
--lib` after building the web app and preparing the bundled command. Browser
tests in `e2e/terminal-command.spec.ts` check notices and recovery actions.
Release builds run `desktop/scripts/check-appimage-command.py IMAGE
BUILT_TERYON --build-newer` under `xvfb-run`. This builds a higher-version test
AppImage, restores the manifests, and removes the fixture afterwards. An
existing higher-version package can instead be supplied with `--newer-image
PATH`. Checks use temporary homes and wait for completed diagnostic records.
They cover first and repeat launches, upgrades, an older launch after an
upgrade, a confirmed conflict notice, execution after closing the window,
and a fake agent making a real call through its bundled command. No model
provider is contacted.

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
