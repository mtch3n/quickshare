import { TrashIcon } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Field, FieldLabel } from "@/components/ui/field"
import { useQuickShare } from "@/hooks/quick-share"
import { type TrustedDevice, api } from "@/lib/tauri"

/** One settings row per trusted device. */
export function TrustedDevices() {
  const { settings, updateSettings } = useQuickShare()
  const trustedDevices = settings?.trustedDevices ?? []

  const handleRemove = (device: TrustedDevice) => {
    api.untrustDevice(device).then(() => {
      const updated = trustedDevices.filter(
        (d) => d.name !== device.name || d.fingerprint !== device.fingerprint
      )
      updateSettings({ trustedDevices: updated })
    })
  }

  if (trustedDevices.length === 0) {
    return (
      <p className="flex items-center text-sm text-muted-foreground">
        No trusted devices yet
      </p>
    )
  }

  return trustedDevices.map((device) => (
    <Field
      key={`${device.name}/${device.fingerprint}`}
      orientation="horizontal"
    >
      <FieldLabel className="min-w-0">
        <span className="truncate">{device.name}</span>
        <Badge variant="secondary">
          {device.fingerprint ? "LocalSend" : "Quick Share"}
        </Badge>
      </FieldLabel>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={`Remove ${device.name}`}
        title="Remove"
        onClick={() => handleRemove(device)}
      >
        <TrashIcon />
      </Button>
    </Field>
  ))
}
