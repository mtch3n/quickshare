import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"

import type { ChannelAction } from "@bindings/ChannelAction"
import type { ChannelMessage } from "@bindings/ChannelMessage"
import type { EndpointInfo } from "@bindings/EndpointInfo"
import type { SendInfo } from "@bindings/SendInfo"
import type { Visibility } from "@bindings/Visibility"
import type { WifiNetwork } from "@bindings/WifiNetwork"

export type Settings = {
  deviceName: string
  visibility: Visibility
  downloadPath: string
  keepRunning: boolean
  fileManagerIntegration: boolean
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
  setFileManagerIntegration: (enabled: boolean) =>
    invoke<void>("set_file_manager_integration", { enabled }),
  startDiscovery: () => invoke<void>("start_discovery"),
  stopDiscovery: () => invoke<void>("stop_discovery"),
  send: (info: SendInfo) => invoke<void>("send_payload", { info }),
  transferAction: (id: string, action: ChannelAction) =>
    invoke<void>("transfer_action", { id, action }),
  takePendingFiles: () => invoke<string[]>("take_pending_files"),
  connectWifi: (network: WifiNetwork) =>
    invoke<void>("connect_wifi", { network }),
}

/** Events emitted by src-tauri/src/main.rs and tray.rs. */
type Events = {
  rs2js_channelmessage: ChannelMessage
  rs2js_endpointinfo: EndpointInfo
  visibility_updated: Visibility
  device_name_updated: string
  send_files: string[]
  send_text: string
  pick_files: null
}

export function on<K extends keyof Events>(
  event: K,
  handler: (payload: Events[K]) => void
): Promise<UnlistenFn> {
  return listen<Events[K]>(event, (e) => handler(e.payload))
}
