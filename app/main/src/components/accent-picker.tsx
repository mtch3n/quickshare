import * as React from "react"
import { cn } from "cn"

import { type Accent, useTheme } from "@/components/theme-provider"
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
  InputGroupText,
} from "@/components/ui/input-group"

/** GNOME's accent colors (libadwaita 1.6). */
const PRESETS = [
  { name: "Blue", color: "#3584e4" },
  { name: "Teal", color: "#2190a4" },
  { name: "Green", color: "#3a944a" },
  { name: "Yellow", color: "#c88800" },
  { name: "Orange", color: "#ed5b00" },
  { name: "Red", color: "#e62d42" },
  { name: "Pink", color: "#d56199" },
  { name: "Purple", color: "#9141ac" },
  { name: "Slate", color: "#6f8396" },
] as const

const SWATCH =
  "size-5 shrink-0 rounded-full outline-offset-2 transition-transform hover:scale-110 aria-checked:outline-2 aria-checked:outline-foreground/70"

export function AccentPicker() {
  const { accent, setAccent, systemAccent } = useTheme()
  const [editing, setEditing] = React.useState(false)
  const custom =
    accent !== "system" &&
    accent !== "neutral" &&
    !PRESETS.some((p) => p.color === accent)

  const option = (value: Accent, label: string) => ({
    role: "radio",
    "aria-checked": accent === value,
    "aria-label": label,
    title: label,
    onClick: () => setAccent(value),
  })

  return (
    <div
      role="radiogroup"
      aria-label="Accent color"
      className="flex flex-wrap items-center gap-1.5"
    >
      <button
        type="button"
        {...option("system", "System")}
        className="mr-1 flex h-6 items-center gap-1.5 rounded-full bg-muted pr-2.5 pl-1.5 text-xs font-medium outline-offset-2 aria-checked:outline-2 aria-checked:outline-foreground/70"
      >
        <span
          className="size-3.5 rounded-full bg-foreground"
          style={systemAccent ? { background: systemAccent } : undefined}
        />
        System
      </button>
      <button
        type="button"
        {...option("neutral", "Neutral")}
        className={cn(SWATCH, "bg-foreground")}
      />
      {PRESETS.map(({ name, color }) => (
        <button
          key={color}
          type="button"
          {...option(color, name)}
          className={SWATCH}
          style={{ background: color }}
        />
      ))}
      <button
        type="button"
        role="radio"
        aria-checked={custom}
        aria-expanded={editing}
        aria-label="Custom color"
        title="Custom color"
        onClick={() => setEditing(!editing)}
        className={SWATCH}
        style={{
          background: custom
            ? accent
            : "conic-gradient(red, yellow, lime, cyan, blue, magenta, red)",
        }}
      />
      {editing && (
        <HexField
          initial={custom ? accent : (systemAccent ?? PRESETS[0].color)}
          onChange={setAccent}
        />
      )}
    </div>
  )
}

/**
 * A custom colour as hex, applied as soon as it's complete. In-page rather
 * than `<input type="color">`, which WebKitGTK shows as GTK's own dialog.
 */
function HexField({
  initial,
  onChange,
}: {
  initial: string
  onChange: (accent: Accent) => void
}) {
  const [text, setText] = React.useState(initial.slice(1))
  const complete = /^[0-9a-f]{6}$/i.test(text)

  return (
    <InputGroup className="mt-1 w-full animate-in duration-200 fade-in-0 slide-in-from-top-1">
      <InputGroupAddon>
        <span
          className="size-4 rounded-full bg-muted-foreground/30"
          style={complete ? { background: `#${text}` } : undefined}
        />
        <InputGroupText>#</InputGroupText>
      </InputGroupAddon>
      <InputGroupInput
        autoFocus
        aria-label="Custom color, hex"
        placeholder="3584e4"
        maxLength={6}
        spellCheck={false}
        className="font-mono"
        value={text}
        aria-invalid={text.length === 6 && !complete}
        onChange={(e) => {
          const next = e.target.value.replace(/^#/, "").slice(0, 6)
          setText(next)
          if (/^[0-9a-f]{6}$/i.test(next)) {
            onChange(`#${next.toLowerCase()}`)
          }
        }}
      />
    </InputGroup>
  )
}
