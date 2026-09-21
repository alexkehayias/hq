// Registers the service worker for the PWA. Loaded by the home page
// (/index.html) so the worker is installed on first visit and controls the
// whole app scope (/). Include via:
//   <script src="/register-sw.js"></script>
if ('serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    navigator.serviceWorker.register('/service-worker.js').catch((error) => {
      console.error('Service worker registration failed:', error);
    });
  });
}
