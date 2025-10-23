/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./src/web/templates/**/*.html",
    "./src/**/*.rs",
  ],
  // Note: Output path is ./src/web/static/css/output.css
  theme: {
    extend: {
      colors: {
        'cooklang': {
          50: '#fff7ed',
          100: '#ffedd5',
          200: '#fed7aa',
          300: '#fdba74',
          400: '#fb923c',
          500: '#f97316',
          600: '#ea580c',
          700: '#c2410c',
          800: '#9a3412',
          900: '#7c2d12',
        },
      },
    },
  },
  plugins: [],
}
