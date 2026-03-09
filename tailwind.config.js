/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./frontend/index.html",
    "./frontend/src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      fontFamily: {
        display: ["Fredoka", "sans-serif"],
        body: ["Nunito", "sans-serif"],
      },
      colors: {
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
        },
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },
        peach: "hsl(var(--peach))",
        lavender: "hsl(var(--lavender))",
        mint: "hsl(var(--mint))",
        cream: "hsl(var(--cream))",
        rose: "hsl(var(--rose))",
        sky: "hsl(var(--sky))",
      },
      boxShadow: {
        cozy: "0 4px 20px -4px hsl(var(--primary) / 0.12), 0 2px 8px -2px hsl(var(--lavender) / 0.15)",
        "cozy-lg": "0 8px 40px -8px hsl(var(--primary) / 0.15), 0 4px 16px -4px hsl(var(--lavender) / 0.2)",
        "cozy-glow": "0 0 30px -5px hsl(var(--primary) / 0.2)",
      },
      keyframes: {
        float: {
          "0%, 100%": { transform: "translateY(0px)" },
          "50%": { transform: "translateY(-8px)" },
        },
        "gentle-bounce": {
          "0%, 100%": { transform: "translateY(0)" },
          "50%": { transform: "translateY(-4px)" },
        },
      },
      animation: {
        float: "float 6s ease-in-out infinite",
        "gentle-bounce": "gentle-bounce 2s ease-in-out infinite",
      },
    },
  },
  plugins: [],
};

