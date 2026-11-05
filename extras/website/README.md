# SCAD Viewer website

Static site for <https://scadviewer.cgeosoft.com>. It uses plain HTML, CSS and JavaScript, with no build step and no library. It sets no cookies. PostHog analytics run only after consent.

The look follows the Portfolio website (`portfolio/extras/website`). The colors follow `src/ui.rs`.

## Files

| Path | Content |
|---|---|
| `public/index.html` | The page |
| `public/style.css` | All styles |
| `public/boot.js` | Sets `.js` before first paint, for the reveal effect |
| `public/analytics.js` | Consent modal and PostHog. The key comes from `deploy.sh` |
| `public/script.js` | OS detection, download links, scroll reveal, screenshot tabs, lightbox |
| `public/viewer.js` | WebGL2 viewer: solid, wire and slice modes |
| `public/demo.js` | Hero loop and the agent demo |
| `public/assets/mesh/` | Generated. Example meshes and `models.json` |
| `public/assets/img/` | Generated. WebP copies of `images/` |
| `public/privacy/index.html` | Privacy policy |
| `public/llms.txt`, `robots.txt`, `sitemap.xml` | Files for crawlers and LLMs |
| `public/_headers` | Security and cache headers |
| `wrangler.toml` | Cloudflare Pages project |

## Demo data

`scripts/website-assets.py` runs the app CLI on the example models. It writes the meshes and the measured values: render time, size, volume, open edges, layers, filament, weight and print time. It also writes the images.

```bash
cargo build --release
scripts/website-assets.py
```

Change the prebuilt parameter values in `MODELS` in that script. The prompts of the agent demo are in `PROMPTS` in `public/demo.js`. Each prompt must name values that `MODELS` has.

The chat text is scripted. The tool names and the numbers are real. The page draws layer outlines from the mesh. The app draws walls, infill and skin.

## Behavior

- Download links come from the GitHub Releases API. If the API fails, the links go to the releases page.
- A `data-since="0.3.0"` mark goes away when the latest release is that version or newer.
- Without WebGL2, images replace the live views.
- With reduced motion, the hero does not loop and nothing fades in.
- The wheel zooms the demo only after a click on the view, so the page scroll stays free.

## Analytics

- `analytics.js` loads PostHog from the EU cloud only after the visitor accepts. Before that, the page makes no request to PostHog.
- PostHog runs with `localStorage` persistence, so the site sets no cookies. Autocapture, session recording, surveys and feature flags are off.
- The events are `$pageview`, `download_clicked` (os, type) and `demo_prompt` (model). Each event has `site: scad-viewer`.
- The choice lasts 12 months. The footer `cookies` button opens the modal again.
- When you add a service, event or storage key, update `public/privacy/index.html`.

Put the key in `extras/website/.env` (git-ignored):

```bash
POSTHOG_API_KEY=phc_...
```

## Preview

```bash
cd extras/website
wrangler pages dev public --port 8788
```

This command also applies `_headers`.

## Deploy

1. Run `wrangler login`, or set `CLOUDFLARE_API_TOKEN`.
2. Make the project one time: `wrangler pages project create scad-viewer --production-branch main`.
3. Deploy: `bun run deploy`, from `extras/website`. This runs `deploy.sh`, which writes the PostHog key into a staged copy. Without a key, analytics stay off and the modal does not show.
4. In the Cloudflare dashboard, open **Workers & Pages > scad-viewer > Custom domains** and add `scadviewer.cgeosoft.com`.

Use `deploy.sh`, not a git build. A git build has no PostHog key.

After the first deploy, check that Cloudflare does not replace `robots.txt`. Open **AI Crawl Control** for the zone and turn off the managed robots.txt if it is on.

The PNG files in `public/` are plain git, not LFS (`.gitattributes`).
