import * as React from "react"
import { CheckIcon, RotateCcwIcon } from "lucide-react"

import { Field, FieldLabel } from "@/components/ui/field"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group"
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
    <Field orientation="horizontal">
      <FieldLabel htmlFor="device-name">Device name</FieldLabel>
      <InputGroup className="w-52">
        <InputGroupInput
          id="device-name"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && hasChanged && !isEmpty) handleSave()
          }}
          placeholder={settings?.deviceName}
          disabled={isLoading}
          maxLength={64}
        />
        <InputGroupAddon align="inline-end">
          {hasChanged && (
            <InputGroupButton
              size="icon-xs"
              aria-label="Save"
              title="Save"
              onClick={handleSave}
              disabled={isEmpty || isLoading}
            >
              <CheckIcon />
            </InputGroupButton>
          )}
          <InputGroupButton
            size="icon-xs"
            aria-label="Use computer name"
            title="Use computer name"
            onClick={handleReset}
            disabled={isLoading}
          >
            <RotateCcwIcon />
          </InputGroupButton>
        </InputGroupAddon>
      </InputGroup>
    </Field>
  )
}
