import { useQuickShare } from "@/hooks/quick-share"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldLabel,
} from "@/components/ui/field"
import { Switch } from "@/components/ui/switch"
import { api } from "@/lib/tauri"

export function ReceiveSettings() {
  const { settings, updateSettings } = useQuickShare()

  const toggleAutoOpenLinks = (enabled: boolean) =>
    api
      .setAutoOpenLinks(enabled)
      .then(() => updateSettings({ autoOpenLinks: enabled }))

  const toggleAutoCopyText = (enabled: boolean) =>
    api
      .setAutoCopyText(enabled)
      .then(() => updateSettings({ autoCopyText: enabled }))

  return (
    <>
      <Field orientation="horizontal">
        <FieldContent>
          <FieldLabel htmlFor="auto-open-links">
            Open received links automatically
          </FieldLabel>
          <FieldDescription>
            Automatically open URLs in your default browser
          </FieldDescription>
        </FieldContent>
        <Switch
          id="auto-open-links"
          checked={settings?.autoOpenLinks ?? false}
          onCheckedChange={toggleAutoOpenLinks}
        />
      </Field>

      <Field orientation="horizontal">
        <FieldContent>
          <FieldLabel htmlFor="auto-copy-text">
            Copy received text automatically
          </FieldLabel>
          <FieldDescription>
            Automatically copy text to your clipboard
          </FieldDescription>
        </FieldContent>
        <Switch
          id="auto-copy-text"
          checked={settings?.autoCopyText ?? false}
          onCheckedChange={toggleAutoCopyText}
        />
      </Field>
    </>
  )
}
