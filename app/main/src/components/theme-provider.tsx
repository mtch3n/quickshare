/* eslint-disable react-refresh/only-export-components */
import * as React from "react"

import { api, on } from "@/lib/tauri"

export type Theme = "dark" | "light" | "system"
/** The desktop's accent, the neutral palette, or a `#rrggbb` color. */
export type Accent = "system" | "neutral" | `#${string}`

const THEME_KEY = "theme"
const ACCENT_KEY = "accent"
const DARK_QUERY = "(prefers-color-scheme: dark)"

type ThemeProviderState = {
  theme: Theme
  setTheme: (theme: Theme) => void
  accent: Accent
  setAccent: (accent: Accent) => void
  /** The desktop's accent as `#rrggbb`, if it has one. */
  systemAccent: string | null
}

const ThemeProviderContext = React.createContext<
  ThemeProviderState | undefined
>(undefined)

function storedTheme(): Theme {
  const value = localStorage.getItem(THEME_KEY)
  return value === "dark" || value === "light" ? value : "system"
}

function storedAccent(): Accent {
  const value = localStorage.getItem(ACCENT_KEY)
  if (value === "neutral" || isHexColor(value)) return value
  return "system"
}

function isHexColor(value: string | null): value is `#${string}` {
  return value !== null && /^#[0-9a-f]{6}$/i.test(value)
}

/** White text unless it would be hard to read on `hex` (WCAG contrast < 3). */
function foregroundOn(hex: string) {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  })
  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b
  return 1.05 / (luminance + 0.05) >= 3
    ? "oklch(0.985 0 0)"
    : "oklch(0.205 0 0)"
}

/** Overrides the stylesheet's neutral primary with `color`, or restores it. */
function applyAccent(color: string | null) {
  const style = document.documentElement.style
  if (color) {
    style.setProperty("--primary", color)
    style.setProperty("--primary-foreground", foregroundOn(color))
    style.setProperty("--ring", color)
  } else {
    style.removeProperty("--primary")
    style.removeProperty("--primary-foreground")
    style.removeProperty("--ring")
  }
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme, setThemeState] = React.useState<Theme>(storedTheme)
  const [accent, setAccentState] = React.useState<Accent>(storedAccent)
  const [systemAccent, setSystemAccent] = React.useState<string | null>(null)

  React.useEffect(() => {
    const media = window.matchMedia(DARK_QUERY)
    const apply = () => {
      const dark = theme === "dark" || (theme === "system" && media.matches)
      document.documentElement.classList.toggle("dark", dark)
    }

    apply()
    media.addEventListener("change", apply)
    return () => media.removeEventListener("change", apply)
  }, [theme])

  React.useEffect(() => {
    api.systemAccentColor().then(setSystemAccent)
    const unlisten = on("system_accent_color", setSystemAccent)
    return () => {
      unlisten.then((f) => f())
    }
  }, [])

  React.useEffect(() => {
    applyAccent(
      accent === "system" ? systemAccent : accent === "neutral" ? null : accent
    )
  }, [accent, systemAccent])

  const value = React.useMemo(
    () => ({
      theme,
      setTheme: (next: Theme) => {
        localStorage.setItem(THEME_KEY, next)
        setThemeState(next)
      },
      accent,
      setAccent: (next: Accent) => {
        localStorage.setItem(ACCENT_KEY, next)
        setAccentState(next)
      },
      systemAccent,
    }),
    [theme, accent, systemAccent]
  )

  return (
    <ThemeProviderContext.Provider value={value}>
      {children}
    </ThemeProviderContext.Provider>
  )
}

export function useTheme() {
  const context = React.useContext(ThemeProviderContext)
  if (!context) {
    throw new Error("useTheme must be used within a ThemeProvider")
  }
  return context
}
