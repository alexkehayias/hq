// Exported theme helpers for pages that render a toggle. The pre-paint theme
// detection (setting .dark before first paint, following OS preference while no
// override is stored) lives in /theme-init.js, which every page loads in <head>
// via <script src="/theme-init.js"></script>.
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
