/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        primary: {
          DEFAULT: '#3b82f6',
          dark: '#1d4ed8',
        },
        background: '#111827',
        surface: {
          DEFAULT: '#1f2937',
          hover: '#374151',
        },
        text: {
          DEFAULT: '#f9fafb',
          muted: '#9ca3af',
        },
        border: '#374151',
      },
    },
  },
  plugins: [],
}
