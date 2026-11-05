# SCAD Viewer website

Static site for <https://scadviewer.cgeosoft.com>. It uses plain HTML, CSS and JavaScript, with no build step and no library. It has no analytics and no cookies.

The look follows the Portfolio website (`portfolio/extras/website`). The colors follow `src/ui.rs`.

## Files

| Path | Content |
|---|---|
| `public/index.html` | The page |
| `public/style.css` | All styles |
| `public/boot.js` | Sets `.js` before first paint, for the reveal effect |
| `public/script.js` | OS detection, download links, scroll reveal, screenshot tabs, lightbox |
| `public/viewer.js` | WebGL2 viewer: solid, wire and slice modes |
| `public/demo.js` | Hero loop and the agent demo |
| `public/assets/mesh/` | Generated. Example meshes and `models.json` |
| `public/assets/img/` | Generated. WebP copies of `images/` |
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

## Preview

```bash
cd extras/website
wrangler pages dev public --port 8788
```

This command also applies `_headers`.

## Deploy

1. Run `wrangler login`, or set `CLOUDFLARE_API_TOKEN`.
2. Make the project one time: `wrangler pages project create scad-viewer --production-branch main`.
3. Deploy: `bun run deploy`, from `extras/website`.
4. In the Cloudflare dashboard, open **Workers & Pages > scad-viewer > Custom domains** and add `scadviewer.cgeosoft.com`.

To build from git instead, connect the GitHub repo and set the build output directory to `extras/website/public`, with no build command and root directory `/`.

The PNG files in `public/` are plain git, not LFS (`.gitattributes`).
