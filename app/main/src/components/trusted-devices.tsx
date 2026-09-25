import { TrashIcon } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Empty } from "@/components/ui/empty"
import { Field, FieldDescription, FieldTitle } from "@/components/ui/field"
import { Item } from "@/components/ui/item"
import { useQuickShare } from "@/hooks/quick-share"
import { api } from "@/lib/tauri"

export function TrustedDevices() {
  const { settings, updateSettings } = useQuickShare()
  const trustedDevices = settings?.trustedDevices ?? []

  const handleRemove = (name: string) => {
    api.untrustDevice(name).then(() => {
      const updated = trustedDevices.filter((d) => d !== name)
      updateSettings({ trustedDevices: updated })
    })
  }

  return (
    <Field>
      <FieldTitle>Trusted devices</FieldTitle>
      <FieldDescription>
        Devices are recognized by name. Anyone nearby can use the same name, so
        only trust devices on networks you trust.
      </FieldDescription>

      {trustedDevices.length === 0 ? (
        <Empty>No trusted devices yet</Empty>
      ) : (
        <div className="mt-4 flex flex-col gap-1">
          {trustedDevices.map((name) => (
            <Item
              key={name}
              className="flex items-center justify-between gap-2"
            >
              <span className="truncate text-sm">{name}</span>
              <Button
                variant="ghost"
                size="icon"
                aria-label={`Remove ${name}`}
                onClick={() => handleRemove(name)}
              >
                <TrashIcon data-icon="inline-start" />
              </Button>
            </Item>
          ))}
        </div>
      )}
    </Field>
  )
}
