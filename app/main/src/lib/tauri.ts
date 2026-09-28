import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"

import type { ChannelAction } from "@bindings/ChannelAction"
import type { ChannelMessage } from "@bindings/ChannelMessage"
import type { EndpointInfo } from "@bindings/EndpointInfo"
import type { SendInfo } from "@bindings/SendInfo"
import type { Visibility } from "@bindings/Visibility"
import type { WifiNetwork } from "@bindings/WifiNetwork"

/** Quick Share devices are trusted by name, LocalSend ones by certificate. */
export type TrustedDevice = {
  name: string
  fingerprint: string | null
}

export type Settings = {
  deviceName: string
  visibility: Visibility
  downloadPath: string
  keepRunning: boolean
  desktopIntegration: boolean
  trustedDevices: TrustedDevice[]
  autoOpenLinks: boolean
  autoCopyText: boolean
}

export type FileSummary = {
  path: string
  /** For a folder, the total of the files inside it. */
  size: number
  isDir: boolean
}

export type Update = {
  version: string
  /** The release page. */
  url: string
}

/** Commands implemented in src-tauri/src/commands.rs. */
export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  setVisibility: (visibility: Visibility) =>
    invoke<void>("set_visibility", { visibility }),
  setDownloadPath: (path: string | null) =>
    invoke<void>("set_download_path", { path }),
  setDeviceName: (name: string | null) =>
    invoke<void>("set_device_name", { name }),
  setKeepRunning: (enabled: boolean) =>
    invoke<void>("set_keep_running", { enabled }),
  setDesktopIntegration: (enabled: boolean) =>
    invoke<void>("set_desktop_integration", { enabled }),
  startDiscovery: () => invoke<void>("start_discovery"),
  stopDiscovery: () => invoke<void>("stop_discovery"),
  send: (info: SendInfo) => invoke<void>("send_payload", { info }),
  transferAction: (id: string, action: ChannelAction) =>
    invoke<void>("transfer_action", { id, action }),
  takePendingFiles: () => invoke<string[]>("take_pending_files"),
  connectWifi: (network: WifiNetwork) =>
    invoke<void>("connect_wifi", { network }),
  trustDevice: (device: TrustedDevice) =>
    invoke<void>("trust_device", { device }),
  untrustDevice: (device: TrustedDevice) =>
    invoke<void>("untrust_device", { device }),
  setAutoOpenLinks: (enabled: boolean) =>
    invoke<void>("set_auto_open_links", { enabled }),
  setAutoCopyText: (enabled: boolean) =>
    invoke<void>("set_auto_copy_text", { enabled }),
  systemAccentColor: () => invoke<string | null>("system_accent_color"),
  /** Through the desktop portal, so the default browser opens it. */
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  checkUpdate: () => invoke<Update | null>("check_update"),
  inspectFiles: (paths: string[]) =>
    invoke<FileSummary[]>("inspect_files", { paths }),
}

/** Events emitted by src-tauri/src/main.rs, tray.rs and accent.rs. */
type Events = {
  rs2js_channelmessage: ChannelMessage
  rs2js_endpointinfo: EndpointInfo
  visibility_updated: Visibility
  device_name_updated: string
  send_files: string[]
  send_text: string
  pick_files: null
  system_accent_color: string | null
}

export function on<K extends keyof Events>(
  event: K,
  handler: (payload: Events[K]) => void
): Promise<UnlistenFn> {
  return listen<Events[K]>(event, (e) => handler(e.payload))
}
