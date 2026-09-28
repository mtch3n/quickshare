import { cn } from "cn"

import { type Accent, useTheme } from "@/components/theme-provider"

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
      <label
        role="radio"
        aria-checked={custom}
        aria-label="Custom color"
        title="Custom color"
        className={cn(SWATCH, "relative cursor-pointer")}
        style={{
          background: custom
            ? accent
            : "conic-gradient(red, yellow, lime, cyan, blue, magenta, red)",
        }}
      >
        <input
          type="color"
          className="absolute inset-0 cursor-pointer opacity-0"
          value={custom ? accent : (systemAccent ?? "#3584e4")}
          onChange={(e) => setAccent(e.target.value as Accent)}
        />
      </label>
    </div>
  )
}
