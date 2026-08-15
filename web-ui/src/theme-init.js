// Runs synchronously in <head> on every page, before first paint, so the
// `.dark` class is on <html> before any content renders (no theme flash).
// Reads the user override from localStorage first, then falls back to the OS
// preference. While no override is stored, live OS theme changes also apply.
//
// Every HTML page must include this via:
//   <script src="/theme-init.js"></script>
(() => {
  const mq = matchMedia('(prefers-color-scheme: dark)');
  const apply = () => {
    const stored = localStorage.getItem('hq-theme');
    const dark = stored === 'dark' || (!stored && mq.matches);
    document.documentElement.classList.toggle('dark', dark);
  };
  apply();
  if (!localStorage.getItem('hq-theme')) {
    mq.addEventListener('change', apply);
  }
})();
