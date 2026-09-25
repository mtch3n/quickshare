import {
  LaptopIcon,
  MonitorSmartphoneIcon,
  SmartphoneIcon,
  TabletIcon,
} from "lucide-react"

import type { DeviceType } from "@bindings/DeviceType"

const ICONS = {
  Phone: SmartphoneIcon,
  Tablet: TabletIcon,
  Laptop: LaptopIcon,
  Unknown: MonitorSmartphoneIcon,
}

export function DeviceIcon({ type }: { type: DeviceType | null | undefined }) {
  const Icon = ICONS[type ?? "Unknown"]
  return <Icon />
}
