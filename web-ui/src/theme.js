// Theme detection — runs before paint to set the .dark class on <html>.
// Checks localStorage (user override) first, then falls back to OS preference.
// This script is intentionally inline in each HTML <head> via the snippet:
//
//   <script>
//     const t = localStorage.getItem('hq-theme');
//     if (t === 'dark' || (!t && matchMedia('(prefers-color-scheme: dark)').matches)) {
//       document.documentElement.classList.add('dark');
//     }
//   </script>
//
// Exported helpers (loaded as a module by pages that have a toggle):
export function isDark() {
  return document.documentElement.classList.contains('dark');
}

export function setDark(dark) {
  const el = document.documentElement;
  if (dark) el.classList.add('dark');
  else el.classList.remove('dark');
  try {
    localStorage.setItem('hq-theme', dark ? 'dark' : 'light');
  } catch {}
}

export function toggleDark() {
  setDark(!isDark());
}