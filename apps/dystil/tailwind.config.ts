/** @type {import('tailwindcss').Config} */
//
// Values here come from agent_docs/design_handoff_home_screen/README.md.
// The app is light-only — there is no darkMode setting and no `.dark` block
// in globals.css. Do not add `dark:` variants.
//
module.exports = {
  content: [
    "./pages/**/*.{ts,tsx}",
    "./components/**/*.{ts,tsx}",
    "./app/**/*.{ts,tsx}",
    "./src/**/*.{ts,tsx}",
  ],
  prefix: "",
  theme: {
    container: {
      center: "true",
      padding: "2rem",
      screens: {
        "2xl": "1400px",
      },
    },
    extend: {
      fontFamily: {
        // Bound to next/font variables in app/layout.tsx, which self-hosts both
        // faces at build time so the app works offline.
        sans: ["var(--font-instrument-sans)", "ui-sans-serif", "-apple-system", "BlinkMacSystemFont", "Segoe UI", "sans-serif"],
        display: ["var(--font-newsreader)", "Iowan Old Style", "Palatino Linotype", "Georgia", "serif"],
        mono: ["ui-monospace", "SFMono-Regular", "SF Mono", "Menlo", "monospace"],
      },
      fontSize: {
        // Stock steps — used by settings, onboarding, auth and components/ui/*.
        xs:    ["var(--text-xs)",   { lineHeight: "1rem" }],
        sm:    ["var(--text-sm)",   { lineHeight: "1.25rem" }],
        base:  ["var(--text-base)", { lineHeight: "1.5rem" }],
        lg:    ["var(--text-lg)",   { lineHeight: "1.75rem" }],
        xl:    ["var(--text-xl)",   { lineHeight: "1.75rem" }],
        "2xl": ["var(--text-2xl)",  { lineHeight: "2rem" }],
        "3xl": ["var(--text-3xl)",  { lineHeight: "2.25rem" }],
        "4xl": ["var(--text-4xl)",  { lineHeight: "2.5rem" }],

        // Handoff steps. Headlines 1.24-1.26, body 1.45-1.6, per the README.
        // Large display sizes also want letter-spacing -.015em to -.02em.
        "display-lg": ["var(--text-display-lg)", { lineHeight: "1.24", letterSpacing: "-0.018em" }],
        display:      ["var(--text-display)",    { lineHeight: "1.26", letterSpacing: "-0.015em" }],
        "display-sm": ["var(--text-display-sm)", { lineHeight: "1.3",  letterSpacing: "-0.01em" }],
        num:          ["var(--text-num)",        { lineHeight: "1" }],
        "num-sm":     ["var(--text-num-sm)",     { lineHeight: "1" }],
        hero:         ["var(--text-hero)",       { lineHeight: "1.5" }],
        title:        ["var(--text-title)",      { lineHeight: "1.4" }],
        "body-lg":    ["var(--text-body-lg)",    { lineHeight: "1.55" }],
        body:         ["var(--text-body)",       { lineHeight: "1.45" }],
        "body-sm":    ["var(--text-body-sm)",    { lineHeight: "1.45" }],
        ui:           ["var(--text-ui)",         { lineHeight: "1.4" }],
        "ui-sm":      ["var(--text-ui-sm)",      { lineHeight: "1.4" }],
        meta:         ["var(--text-meta)",       { lineHeight: "1.4" }],
        label:        ["var(--text-label)",      { lineHeight: "1.3" }],
        "label-sm":   ["var(--text-label-sm)",   { lineHeight: "1.3" }],
      },
      colors: {
        // --- Handoff palette, addressable directly ---------------------------
        // green = you can act / it is local and safe
        // marigold = something needs your judgement
        // Never green for navigation. At most one green element per region.
        ground: "hsl(var(--ground))",
        chrome: "hsl(var(--chrome))",
        paper: "hsl(var(--paper))",
        recessed: "hsl(var(--recessed))",
        ink: {
          DEFAULT: "hsl(var(--ink))",
          2: "hsl(var(--ink-2))",
          3: "hsl(var(--ink-3))",
        },
        line: {
          DEFAULT: "hsl(var(--line))",
          2: "hsl(var(--line-2))",
          "2b": "hsl(var(--line-2b))",
          "2c": "hsl(var(--line-2c))",
          3: "hsl(var(--line-3))",
        },
        green: {
          deep: "hsl(var(--green-deep))",
          "deep-hover": "hsl(var(--green-deep-hover))",
          mark: "hsl(var(--green-mark))",
          mid: "hsl(var(--green-mid))",
        },
        sage: {
          DEFAULT: "hsl(var(--sage))",
          dark: "hsl(var(--sage-dark))",
          tint: "hsl(var(--sage-tint))",
          border: "hsl(var(--sage-border))",
          hover: "hsl(var(--sage-hover))",
        },
        marigold: {
          DEFAULT: "hsl(var(--marigold))",
          text: "hsl(var(--marigold-text))",
          tint: "hsl(var(--marigold-tint))",
          hover: "hsl(var(--marigold-hover))",
        },
        chevron: "hsl(var(--chevron))",

        // --- Semantic aliases ------------------------------------------------
        // Consumed by settings, onboarding, auth and components/ui/*, which
        // inherit the new palette through these without changes.
        border: "hsl(var(--border))",
        input: {
          DEFAULT: "hsl(var(--input))",
          focus: "hsl(var(--input-focus))",
        },
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        surface: {
          DEFAULT: "hsl(var(--surface))",
          secondary: "hsl(var(--surface-secondary))",
          tertiary: "hsl(var(--surface-tertiary))",
        },
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
          hover: "hsl(var(--primary-hover))",
          muted: "hsl(var(--primary-muted))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
          hover: "hsl(var(--secondary-hover))",
        },
        success: {
          DEFAULT: "hsl(var(--success))",
          foreground: "hsl(var(--success-foreground))",
          muted: "hsl(var(--success-muted))",
        },
        warning: {
          DEFAULT: "hsl(var(--warning))",
          foreground: "hsl(var(--warning-foreground))",
          muted: "hsl(var(--warning-muted))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
          hover: "hsl(var(--destructive-hover))",
          muted: "hsl(var(--destructive-muted))",
        },
        info: {
          DEFAULT: "hsl(var(--info))",
          foreground: "hsl(var(--info-foreground))",
          muted: "hsl(var(--info-muted))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
          hover: "hsl(var(--accent-hover))",
        },
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
          hover: "hsl(var(--card-hover))",
        },
        popover: {
          DEFAULT: "hsl(var(--popover))",
          foreground: "hsl(var(--popover-foreground))",
        },
        text: {
          primary: "hsl(var(--text-primary))",
          secondary: "hsl(var(--text-secondary))",
          tertiary: "hsl(var(--text-tertiary))",
          disabled: "hsl(var(--text-disabled))",
        },
      },
      borderRadius: {
        // Handoff radius scale.
        track: "2px",
        badge: "5px",
        strip: "7px",
        icon: "8px",
        tile: "9px",
        button: "10px",
        panel: "13px",
        card: "14px",
        pill: "20px",
        // Kept so components/ui/* primitives keep resolving.
        lg: "var(--radius)",
        md: "calc(var(--radius) - 3px)",
        sm: "calc(var(--radius) - 5px)",
      },
      boxShadow: {
        "card-hover": "0 6px 18px -10px rgba(40,34,20,.2)",
        hero: "0 3px 14px -6px rgba(40,34,20,.16)",
        "hero-hover": "0 10px 28px -12px rgba(40,34,20,.26)",
        primary: "0 2px 6px -2px rgba(14,79,60,.45)",
      },
      keyframes: {
        blink: {
          "0%, 100%": { opacity: "1" },
          "50%": { opacity: "0" },
        },
        "accordion-down": {
          from: { height: "0" },
          to: { height: "var(--radix-accordion-content-height)" },
        },
        "accordion-up": {
          from: { height: "var(--radix-accordion-content-height)" },
          to: { height: "0" },
        },
        pulse: {
          "0%, 100%": { opacity: "1" },
          "50%": { opacity: ".5" },
        },
        rainbow: {
          "0%": { "background-position": "0%" },
          "100%": { "background-position": "200%" },
        },
        "owned-browser-load": {
          "0%": { transform: "translateX(-100%)" },
          "100%": { transform: "translateX(400%)" },
        },
        // --- Handoff animations ---
        halo: {
          "0%, 100%": { opacity: ".22", transform: "scale(1)" },
          "50%": { opacity: ".5", transform: "scale(1.22)" },
        },
        "dot-pulse": {
          "0%, 100%": { opacity: "1" },
          "50%": { opacity: ".35" },
        },
        crawl: {
          "0%": { transform: "translateX(-100%)" },
          "100%": { transform: "translateX(400%)" },
        },
        rise: {
          from: { opacity: "0", transform: "translateY(8px)" },
          to: { opacity: "1", transform: "translateY(0)" },
        },
      },
      animation: {
        "accordion-down": "accordion-down 0.2s ease-out",
        "accordion-up": "accordion-up 0.2s ease-out",
        pulse: "pulse 2s cubic-bezier(0.4, 0, 0.6, 1) infinite",
        rainbow: "rainbow var(--speed, 2s) infinite linear",
        "owned-browser-load": "owned-browser-load 1.1s ease-in-out infinite",
        halo: "halo 3.8s ease-in-out infinite",
        "dot-pulse": "dot-pulse 1.5s ease-in-out infinite",
        crawl: "crawl 2.2s linear infinite",
        rise: "rise 0.4s ease",
      },
    },
  },
  plugins: [require("tailwindcss-animate"), require("@tailwindcss/typography")],
};
