# Building

The repository is a Cargo workspace with two crates, plus a web frontend:

- `core_lib`: the Quick Share protocol (discovery, encryption, transfers).
- `app/main/src-tauri`: the Tauri desktop app built on `core_lib`.
- `app/main`: the React + shadcn/ui frontend.

## Requirements

- Rust: `rustup` installs the version pinned in `rust-toolchain.toml`.
- Node.js 22+ and pnpm (the version is pinned in `app/main/package.json`).
- `protoc` (protobuf compiler)
- WebKitGTK 4.1, GTK 3, D-Bus and librsvg development files

On Arch:

```bash
sudo pacman -S --needed protobuf webkit2gtk-4.1 gtk3 librsvg dbus
```

On Debian/Ubuntu:

```bash
sudo apt install protobuf-compiler libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libdbus-1-dev
```

## Run and build

All commands run from `app/main`:

```bash
pnpm install
pnpm tauri dev                        # run with hot reload
pnpm tauri build --bundles appimage   # build the AppDir into target/release/bundle/appimage
```

Then repack it without the bundled Wayland libraries (they make WebKitGTK
abort on current distributions):

```bash
src-tauri/linux/repack-appimage.sh ../../target/release/bundle/appimage/rquickshare.AppDir RQuickShare.AppImage
```

## Checks

These are the checks CI runs:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                # also regenerates core_lib/bindings (TypeScript types)
cd app/main && pnpm lint && pnpm build
```

## Icons

The icons are drawn in `app/main/src-tauri/icons/src/*.svg`. After editing them,
run `app/main/src-tauri/icons/render.sh` to regenerate the PNGs.
