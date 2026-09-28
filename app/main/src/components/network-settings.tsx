import * as React from "react"

import { Field, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { toast } from "@/components/ui/toast"
import { useQuickShare } from "@/hooks/quick-share"
import { api } from "@/lib/tauri"

/** Fixed ports, for firewalls. Blank means automatic / the default. */
export function NetworkSettings() {
  const { settings, updateSettings } = useQuickShare()
  if (!settings) return null

  const save = (port: number | null, localsendPort: number | null) =>
    api
      .setPorts(port, localsendPort)
      .then(() => {
        updateSettings({ port, localsendPort })
        toast.add({
          title: "Restart QuickShare to use the new port",
          type: "success",
        })
      })
      .catch((e) =>
        toast.add({
          title: "Couldn't change the port",
          description: String(e),
          type: "error",
        })
      )

  return (
    <>
      <PortField
        id="quick-share-port"
        label="Quick Share port"
        placeholder="Automatic"
        value={settings.port}
        onSave={(port) => save(port, settings.localsendPort)}
      />
      <PortField
        id="localsend-port"
        label="LocalSend port"
        placeholder="53317"
        value={settings.localsendPort}
        onSave={(port) => save(settings.port, port)}
      />
    </>
  )
}

function PortField({
  id,
  label,
  placeholder,
  value,
  onSave,
}: {
  id: string
  label: string
  placeholder: string
  value: number | null
  onSave: (port: number | null) => void
}) {
  const [text, setText] = React.useState(value?.toString() ?? "")
  const parsed = text.trim() === "" ? null : Number(text)
  const valid = parsed === null || (Number.isInteger(parsed) && parsed <= 65535)

  const commit = () => {
    if (!valid) {
      setText(value?.toString() ?? "")
      return
    }
    if (parsed !== value) onSave(parsed)
  }

  return (
    <Field orientation="horizontal">
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Input
        id={id}
        className="w-28 text-right tabular-nums"
        inputMode="numeric"
        placeholder={placeholder}
        value={text}
        aria-invalid={!valid}
        onChange={(e) => setText(e.target.value.replace(/\D/g, ""))}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
    </Field>
  )
}
