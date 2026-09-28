import * as React from "react"
import { getVersion } from "@tauri-apps/api/app"
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart"
import { open } from "@tauri-apps/plugin-dialog"
import { openUrl } from "@tauri-apps/plugin-opener"
import { ArrowLeftIcon, MonitorIcon, MoonIcon, SunIcon } from "lucide-react"

import { type Theme, useTheme } from "@/components/theme-provider"
import { Button } from "@/components/ui/button"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldSeparator,
  FieldTitle,
} from "@/components/ui/field"
import { Switch } from "@/components/ui/switch"
import { toast } from "@/components/ui/toast"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { DeviceNameField } from "@/components/device-name-field"
import { ReceiveSettings } from "@/components/receive-settings"
import { TrustedDevices } from "@/components/trusted-devices"
import { useQuickShare } from "@/hooks/quick-share"
import { api } from "@/lib/tauri"

const REPOSITORY = "https://github.com/mtch3n/quickshare"

function reportError(title: string) {
  return (e: unknown) =>
    toast.add({ title, description: String(e), type: "error" })
}

export function SettingsPage({ onBack }: { onBack: () => void }) {
  const { settings, updateSettings } = useQuickShare()
  const { theme, setTheme } = useTheme()
  const [autostart, setAutostart] = React.useState(false)
  const [version, setVersion] = React.useState("")

  React.useEffect(() => {
    isEnabled().then(setAutostart)
    getVersion().then(setVersion)
  }, [])

  const chooseDownloadFolder = async () => {
    const path = await open({
      directory: true,
      defaultPath: settings?.downloadPath,
    })
    if (typeof path !== "string") return
    await api.setDownloadPath(path)
    updateSettings({ downloadPath: path })
  }

  const toggleAutostart = (enabled: boolean) =>
    (enabled ? enable() : disable())
      .then(() => setAutostart(enabled))
      .catch(reportError("Couldn't change start at login"))

  const toggleKeepRunning = (enabled: boolean) =>
    api
      .setKeepRunning(enabled)
      .then(() => updateSettings({ keepRunning: enabled }))

  const toggleIntegration = (enabled: boolean) =>
    api
      .setFileManagerIntegration(enabled)
      .then(() => updateSettings({ fileManagerIntegration: enabled }))
      .catch(reportError("Couldn't change the file manager menu"))

  return (
    <div className="flex min-h-full flex-col gap-4 px-4 pb-4">
      <header className="flex items-center gap-2">
        <Button variant="ghost" size="icon" aria-label="Back" onClick={onBack}>
          <ArrowLeftIcon />
        </Button>
        <h1 className="font-heading text-base font-semibold">Settings</h1>
      </header>

      <FieldGroup>
        <DeviceNameField />

        <FieldSeparator />

        <Field>
          <FieldTitle>Download folder</FieldTitle>
          <FieldDescription className="truncate select-text">
            {settings?.downloadPath}
          </FieldDescription>
          <div className="flex gap-2">
            <Button variant="outline" size="sm" onClick={chooseDownloadFolder}>
              Change
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={async () => {
                await api.setDownloadPath(null)
                updateSettings(await api.getSettings())
              }}
            >
              Use Downloads
            </Button>
          </div>
        </Field>

        <FieldSeparator />

        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel htmlFor="keep-running">
              Run in the background
            </FieldLabel>
            <FieldDescription>
              Keep receiving files after closing the window. Reopen it from the
              tray icon.
            </FieldDescription>
          </FieldContent>
          <Switch
            id="keep-running"
            checked={settings?.keepRunning ?? true}
            onCheckedChange={toggleKeepRunning}
          />
        </Field>

        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel htmlFor="autostart">Start at login</FieldLabel>
            <FieldDescription>Starts hidden in the tray.</FieldDescription>
          </FieldContent>
          <Switch
            id="autostart"
            checked={autostart}
            onCheckedChange={toggleAutostart}
          />
        </Field>

        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel htmlFor="integration">File manager menu</FieldLabel>
            <FieldDescription>
              Adds "Send with Quick Share" to Files, Dolphin and Nemo. Files
              needs the nautilus-python package.
            </FieldDescription>
          </FieldContent>
          <Switch
            id="integration"
            checked={settings?.fileManagerIntegration ?? false}
            onCheckedChange={toggleIntegration}
          />
        </Field>

        <FieldSeparator />

        <ReceiveSettings />

        <FieldSeparator />

        <TrustedDevices />

        <FieldSeparator />

        <Field>
          <FieldTitle>Appearance</FieldTitle>
          <ToggleGroup
            variant="outline"
            value={[theme]}
            onValueChange={(value) => value[0] && setTheme(value[0] as Theme)}
          >
            <ToggleGroupItem value="system">
              <MonitorIcon data-icon="inline-start" />
              System
            </ToggleGroupItem>
            <ToggleGroupItem value="light">
              <SunIcon data-icon="inline-start" />
              Light
            </ToggleGroupItem>
            <ToggleGroupItem value="dark">
              <MoonIcon data-icon="inline-start" />
              Dark
            </ToggleGroupItem>
          </ToggleGroup>
        </Field>
      </FieldGroup>

      <footer className="mt-auto flex items-center justify-between text-sm text-muted-foreground">
        <span>RQuickShare {version}</span>
        <Button variant="link" size="sm" onClick={() => openUrl(REPOSITORY)}>
          Source code
        </Button>
      </footer>
    </div>
  )
}
