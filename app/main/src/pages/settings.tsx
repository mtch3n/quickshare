import * as React from "react"
import { getVersion } from "@tauri-apps/api/app"
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart"
import { open } from "@tauri-apps/plugin-dialog"
import {
  CheckIcon,
  DownloadIcon,
  ExternalLinkIcon,
  FolderOpenIcon,
  MonitorIcon,
  MoonIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  SunIcon,
} from "lucide-react"

import { AccentPicker } from "@/components/accent-picker"
import { type Theme, useTheme } from "@/components/theme-provider"
import { Button } from "@/components/ui/button"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldLabel,
} from "@/components/ui/field"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { toast } from "@/components/ui/toast"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { DeviceNameField } from "@/components/device-name-field"
import { ReceiveSettings } from "@/components/receive-settings"
import { TrustedDevices } from "@/components/trusted-devices"
import { useQuickShare } from "@/hooks/quick-share"
import { type Update, api } from "@/lib/tauri"

const REPOSITORY = "https://github.com/mtch3n/quickshare"
/** Every release carries QuickShare-browser-extension.zip. */
const RELEASES = `${REPOSITORY}/releases/latest`

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
    <div className="flex min-h-full flex-col gap-6 px-4 pb-4">
      <SettingsSection title="This device">
        <DeviceNameField />

        <Field orientation="horizontal">
          <FieldLabel htmlFor="download-folder">Download folder</FieldLabel>
          <InputGroup className="w-52">
            <InputGroupInput
              id="download-folder"
              readOnly
              value={settings?.downloadPath ?? ""}
              title={settings?.downloadPath}
              className="truncate"
            />
            <InputGroupAddon align="inline-end">
              <InputGroupButton
                size="icon-xs"
                aria-label="Change folder"
                title="Change folder"
                onClick={chooseDownloadFolder}
              >
                <FolderOpenIcon />
              </InputGroupButton>
              <InputGroupButton
                size="icon-xs"
                aria-label="Use Downloads"
                title="Use Downloads"
                onClick={async () => {
                  await api.setDownloadPath(null)
                  updateSettings(await api.getSettings())
                }}
              >
                <RotateCcwIcon />
              </InputGroupButton>
            </InputGroupAddon>
          </InputGroup>
        </Field>
      </SettingsSection>

      <SettingsSection title="Receiving">
        <ReceiveSettings />
      </SettingsSection>

      <SettingsSection
        title="Trusted devices"
        footer="LocalSend devices are recognised by their certificate. Quick Share ones only by name, so trust those on networks you trust."
      >
        <TrustedDevices />
      </SettingsSection>

      <SettingsSection title="System">
        <Field orientation="horizontal">
          <FieldLabel htmlFor="keep-running">Run in the background</FieldLabel>
          <Switch
            id="keep-running"
            checked={settings?.keepRunning ?? true}
            onCheckedChange={toggleKeepRunning}
          />
        </Field>

        <Field orientation="horizontal">
          <FieldLabel htmlFor="autostart">Start at login</FieldLabel>
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
              File manager menus and a GNOME tile.
            </FieldDescription>
          </FieldContent>
          <Switch
            id="integration"
            checked={settings?.desktopIntegration ?? false}
            onCheckedChange={toggleIntegration}
          />
        </Field>

        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel>Browser extension</FieldLabel>
            <FieldDescription>Needs desktop integrations.</FieldDescription>
          </FieldContent>
          <Button
            variant="secondary"
            size="sm"
            onClick={() =>
              api
                .openUrl(RELEASES)
                .catch(reportError("Couldn't open the release page"))
            }
          >
            <DownloadIcon data-icon="inline-start" />
            Download
          </Button>
        </Field>
      </SettingsSection>

      <SettingsSection title="Appearance">
        <Field orientation="horizontal">
          <FieldLabel>Theme</FieldLabel>
          <ToggleGroup
            spacing={0}
            className="gap-0.5 rounded-lg bg-muted p-0.5"
            value={[theme]}
            onValueChange={(value) => value[0] && setTheme(value[0] as Theme)}
          >
            {THEMES.map(({ value, label, Icon }) => (
              <ToggleGroupItem
                key={value}
                value={value}
                size="sm"
                aria-label={label}
                title={label}
                className="rounded-md! text-muted-foreground hover:bg-transparent aria-pressed:bg-background aria-pressed:text-foreground aria-pressed:shadow-xs"
              >
                <Icon />
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        </Field>

        <Field>
          <FieldLabel>Accent color</FieldLabel>
          <AccentPicker />
        </Field>
      </SettingsSection>

      <SettingsSection title="About">
        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel>QuickShare</FieldLabel>
            <FieldDescription>Version {version}</FieldDescription>
          </FieldContent>
          <UpdateCheck />
        </Field>

        <Field orientation="horizontal">
          <FieldLabel>Source code</FieldLabel>
          <Button
            variant="secondary"
            size="sm"
            onClick={() =>
              api.openUrl(REPOSITORY).catch(reportError("Couldn't open GitHub"))
            }
          >
            <ExternalLinkIcon data-icon="inline-start" />
            GitHub
          </Button>
        </Field>
      </SettingsSection>
    </div>
  )
}

/** Checks GitHub on request, then offers the release page if there's news. */
function UpdateCheck() {
  const [state, setState] = React.useState<
    | { status: "idle" | "checking" | "current" }
    | { status: "available"; update: Update }
  >({ status: "idle" })

  const check = () => {
    setState({ status: "checking" })
    api
      .checkUpdate()
      .then((update) =>
        setState(
          update ? { status: "available", update } : { status: "current" }
        )
      )
      .catch((e) => {
        setState({ status: "idle" })
        reportError("Couldn't check for updates")(e)
      })
  }

  if (state.status === "available") {
    return (
      <Button
        size="sm"
        className="animate-in duration-200 fade-in-0"
        onClick={() =>
          api
            .openUrl(state.update.url)
            .catch(reportError("Couldn't open the release page"))
        }
      >
        <DownloadIcon data-icon="inline-start" />
        Get {state.update.version}
      </Button>
    )
  }
  if (state.status === "current") {
    return (
      <span className="flex animate-in items-center gap-1.5 text-sm text-muted-foreground duration-200 fade-in-0">
        <CheckIcon className="size-4" />
        Up to date
      </span>
    )
  }
  return (
    <Button
      variant="secondary"
      size="sm"
      disabled={state.status === "checking"}
      onClick={check}
    >
      {state.status === "checking" ? (
        <Spinner data-icon="inline-start" />
      ) : (
        <RefreshCwIcon data-icon="inline-start" />
      )}
      Check for updates
    </Button>
  )
}

const THEMES = [
  { value: "system", label: "System", Icon: MonitorIcon },
  { value: "light", label: "Light", Icon: SunIcon },
  { value: "dark", label: "Dark", Icon: MoonIcon },
] as const

/** A titled card of settings rows, separated by soft dividers. */
function SettingsSection({
  title,
  footer,
  children,
}: {
  title: string
  footer?: string
  children: React.ReactNode
}) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="px-1 text-sm font-medium text-muted-foreground">
        {title}
      </h2>
      <div className="flex flex-col divide-y divide-border/60 rounded-xl bg-muted/40 *:min-h-12 *:px-3 *:py-2.5 *:data-[orientation=horizontal]:items-center! dark:bg-card">
        {children}
      </div>
      {footer && <p className="px-1 text-xs text-muted-foreground">{footer}</p>}
    </section>
  )
}
