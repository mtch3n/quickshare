<div align="center">
  <img src="app/main/src-tauri/icons/src/rquickshare.svg" width="96" alt="" />
  <h1>RQuickShare</h1>
  <p><strong>Quick Share (Nearby Share) for Linux</strong></p>

[![Build](https://github.com/mtch3n/quickshare/actions/workflows/build.yml/badge.svg)](https://github.com/mtch3n/quickshare/actions/workflows/build.yml)
</div>

Send and receive files, links and text between Linux and Android over Google's
Quick Share protocol.

This is a maintained fork of [Martichou/rquickshare](https://github.com/Martichou/rquickshare),
which is no longer updated.

![RQuickShare in light and dark mode](.github/demo.png)

## Install

Download the AppImage from [Releases](https://github.com/mtch3n/quickshare/releases):

- **Latest release**: tagged versions.
- **Nightly**: built from every commit on `master`.

```bash
chmod +x RQuickShare-*.AppImage
./RQuickShare-*.AppImage
```

AppImages need FUSE 2 (`fuse2` on Arch, `libfuse2` on Debian/Ubuntu).

## Using it

- **Receive**: leave the app (or its tray icon) running and keep *Visible to
  everyone* on. Each incoming transfer asks for your confirmation. Check that
  the PIN matches the one on the phone.
- **Send**: drop files on the window or click *Choose files*, then pick a
  nearby device.
- **From your file manager**: turn on *Settings → File manager menu*. This
  adds *Send with Quick Share* to the right-click menu of Files (Nautilus),
  Dolphin and Nemo. Files needs the `nautilus-python` package
  (`python-nautilus` on Arch). You can also run `RQuickShare-*.AppImage --send FILE...`.
- **In the background**: closing the window keeps the app in the tray. On GNOME
  that needs the *AppIndicator and KStatusNotifierItem Support* extension.

## Protocol support

| Feature | Status |
| --- | --- |
| Receive files, links, text, Wi-Fi credentials | ✅ |
| Send files | ✅ |
| Send text / links | ❌ not yet |
| Wi-Fi LAN transport (mDNS + TCP) | ✅ |
| Bluetooth LE wake-up, so phones notice this computer | ✅ |
| Bluetooth data transfer (BLE / RFCOMM) | ❌ not yet, see below |
| Wi-Fi Direct / hotspot / WebRTC upgrades | ❌ |
| "Your devices" / contacts-only visibility | ❌ needs Google account certificates |
| AirDrop | ❌ see below |

**Bluetooth.** Transfers currently run over Wi-Fi LAN only, so both devices must
be on the same network with mDNS allowed. Bluetooth is only used to wake
Android up so it looks for us. Receiving over Bluetooth LE has been shown to
work with Pixel phones in another fork. It is also the fix for phones that
turn Wi-Fi off while *Share with Apple devices* is enabled (Pixel 10, Galaxy S26).
It is planned here.

**AirDrop.** Not supported, and not realistic in this app. It needs Apple's
AWDL Wi-Fi link, which on Linux only works with a few Wi-Fi chips that
support monitor mode and injection, plus Apple-signed certificates for
anything except "Everyone" mode. Pixel's AirDrop interop didn't change the
Quick Share protocol.

## FAQ

**My phone doesn't see my computer.** Both devices must be on the same Wi-Fi,
and the network must allow mDNS (guest and public networks often don't). Turn
Bluetooth on: the app watches for phones that are sharing and re-announces
itself.

**My computer doesn't see my phone.** Android only announces itself while its
Quick Share screen is open and set to *Everyone*. Keep Bluetooth on so the
phone receives our wake-up signal.

**Bluetooth audio or my mouse stutters while the app runs.** The app looks for
phones that are sharing. With `Experimental = true` in `/etc/bluetooth/main.conf`
(then restart `bluetooth.service`), BlueZ lets it listen passively, which doesn't
compete with your other devices.

**My firewall blocks transfers.** Pin the port in
`~/.local/share/dev.mandre.rquickshare/.settings.json`, for example
`"port": 12345`, then allow that TCP port and mDNS (UDP 5353).

**The window is blank.** WebKitGTK's GPU renderer is already disabled by
default. If it still happens, try `WEBKIT_DISABLE_COMPOSITING_MODE=1 ./RQuickShare-*.AppImage`.

## Building

See [BUILD.md](BUILD.md).

## Credits

- [Martichou/rquickshare](https://github.com/Martichou/rquickshare), the project this is based on
- [grishka/NearDrop](https://github.com/grishka/NearDrop) and
  [vicr123/QNearbyShare](https://github.com/vicr123/QNearbyShare), for documenting the protocol
- [nozwock/packet](https://github.com/nozwock/packet), which inspired the file manager integration

Licensed under the GNU GPL v3.
