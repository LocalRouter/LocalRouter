import { useState, useEffect } from "react"

type Theme = "light" | "dark" | "system"

function getSystemTheme(): "light" | "dark" {
  if (typeof window === "undefined") return "light"
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"
}

function applyTheme(theme: Theme) {
  const root = document.documentElement
  const effectiveTheme = theme === "system" ? getSystemTheme() : theme

  if (effectiveTheme === "dark") {
    root.classList.add("dark")
  } else {
    root.classList.remove("dark")
  }
}

export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(() => {
    if (typeof window === "undefined") return "system"
    try {
      const stored = localStorage.getItem("website-theme")
      return stored === "light" || stored === "dark" || stored === "system" ? stored : "system"
    } catch {
      // Storage may be disabled in private browsing or an embedded demo.
      return "system"
    }
  })

  // Apply theme on mount and when theme changes
  useEffect(() => {
    applyTheme(theme)
    try {
      localStorage.setItem("website-theme", theme)
    } catch {
      // Theme changes still work for this session when storage is unavailable.
    }
  }, [theme])

  const [systemTheme, setSystemTheme] = useState(getSystemTheme)

  // Listen for system theme changes when in "system" mode
  useEffect(() => {
    if (theme !== "system") return

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)")
    const handleChange = () => {
      setSystemTheme(getSystemTheme())
      applyTheme("system")
    }
    handleChange()

    mediaQuery.addEventListener("change", handleChange)
    return () => mediaQuery.removeEventListener("change", handleChange)
  }, [theme])

  const setTheme = (newTheme: Theme) => {
    setThemeState(newTheme)
  }

  const toggleTheme = () => {
    // Cycle through: system -> light -> dark -> system
    const next: Record<Theme, Theme> = {
      system: "light",
      light: "dark",
      dark: "system",
    }
    setTheme(next[theme])
  }

  const effectiveTheme = theme === "system" ? systemTheme : theme

  return {
    theme,
    effectiveTheme,
    setTheme,
    toggleTheme,
  }
}
