import * as React from "react"
import { getVersion } from "@tauri-apps/api/app"
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart"
import { open } from "@tauri-apps/plugin-dialog"
import { openPath, openUrl } from "@tauri-apps/plugin-opener"
import { MonitorIcon, MoonIcon, SunIcon } from "lucide-react"

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

export function SettingsPage() {
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
      .setDesktopIntegration(enabled)
      .then(() => updateSettings({ desktopIntegration: enabled }))
      .catch(reportError("Couldn't change the desktop integrations"))

  return (
    <div className="flex min-h-full flex-col gap-4 px-4 pb-4">
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
            <FieldLabel htmlFor="integration">Desktop integrations</FieldLabel>
            <FieldDescription>
              "Send with Quick Share" in Files, Dolphin and Nemo (Files needs
              nautilus-python), a Quick Share tile in GNOME's quick settings
              (after logging in again), and the browser extension's link to this
              app.
            </FieldDescription>
          </FieldContent>
          <Switch
            id="integration"
            checked={settings?.desktopIntegration ?? false}
            onCheckedChange={toggleIntegration}
          />
        </Field>

        {settings?.desktopIntegration && (
          <Field>
            <FieldTitle>Browser extension</FieldTitle>
            <FieldDescription>
              Sends the page, a link or selected text from Chrome, Chromium,
              Brave, Edge or Vivaldi. Open the browser's extensions page, turn
              on Developer mode, choose Load unpacked, and pick this folder.
            </FieldDescription>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() =>
                  openPath(settings.browserExtensionDir).catch(
                    reportError("Couldn't open the folder")
                  )
                }
              >
                Open folder
              </Button>
            </div>
          </Field>
        )}

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
