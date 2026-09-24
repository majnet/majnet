// Installability only — deliberately does NOT cache.
//
// Chrome gates the install prompt on a service worker with a fetch handler, so
// there has to be one. Caching is the part to say no to: every view here polls a
// live API, so a cached shell buys nothing offline (the page would render empty)
// and risks serving stale JS against a changed backend. Pass through and let
// nginx + HTTP caching do their job.
self.addEventListener('fetch', () => {})
