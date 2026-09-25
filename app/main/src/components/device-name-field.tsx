import * as React from "react"
import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldTitle } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { toast } from "@/components/ui/toast"
import { useQuickShare } from "@/hooks/quick-share"
import { api } from "@/lib/tauri"

export function DeviceNameField() {
  const { settings } = useQuickShare()
  const [value, setValue] = React.useState<string>("")
  const [isLoading, setIsLoading] = React.useState(false)

  React.useEffect(() => {
    // Sync local value when settings change (e.g., after reset from another source)
    if (value !== (settings?.deviceName ?? "")) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setValue(settings?.deviceName ?? "")
    }
  }, [settings?.deviceName, value])

  const hasChanged = value !== (settings?.deviceName ?? "")
  const isEmpty = value.trim() === ""

  const handleSave = async () => {
    if (isEmpty) {
      toast.add({
        title: "Device name cannot be empty",
        type: "error",
      })
      return
    }

    setIsLoading(true)
    try {
      await api.setDeviceName(value)
      toast.add({
        title: "Device name updated",
        type: "success",
      })
    } catch (e) {
      toast.add({
        title: "Couldn't update device name",
        description: String(e),
        type: "error",
      })
    } finally {
      setIsLoading(false)
    }
  }

  const handleReset = async () => {
    setIsLoading(true)
    try {
      await api.setDeviceName(null)
      setValue("")
      toast.add({
        title: "Using computer name",
        type: "success",
      })
    } catch (e) {
      toast.add({
        title: "Couldn't reset device name",
        description: String(e),
        type: "error",
      })
    } finally {
      setIsLoading(false)
    }
  }

  return (
    <Field>
      <FieldTitle>Device name</FieldTitle>
      <FieldDescription>
        How this computer appears on other devices.
      </FieldDescription>
      <div className="flex gap-2">
        <Input
          type="text"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder={settings?.deviceName}
          disabled={isLoading}
          maxLength={64}
        />
        <Button
          variant="outline"
          size="sm"
          onClick={handleSave}
          disabled={!hasChanged || isEmpty || isLoading}
        >
          Save
        </Button>
      </div>
      <Button
        variant="ghost"
        size="sm"
        onClick={handleReset}
        disabled={isLoading}
      >
        Use computer name
      </Button>
    </Field>
  )
}
