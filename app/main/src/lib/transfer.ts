import type { Transfer } from "@/hooks/quick-share"
import { fileName, plural } from "@/lib/format"

/** "photo.jpg", "3 files", "a link", ... */
export function describeContent(transfer: Transfer) {
  const meta = transfer.meta
  switch (meta?.text_type) {
    case "Url":
      return "a link"
    case "Text":
      return "some text"
    case "Wifi":
      return meta.wifi?.ssid
        ? `the Wi-Fi network "${meta.wifi.ssid}"`
        : "a Wi-Fi network"
  }

  const files = meta?.files ?? []
  return files.length === 1 ? fileName(files[0]) : plural(files.length, "file")
}
