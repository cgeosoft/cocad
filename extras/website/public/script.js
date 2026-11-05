/**
 * Page behavior: OS detection, download links from the latest GitHub release,
 * scroll reveal, screenshot tabs and the lightbox.
 */
(function () {
  "use strict";

  const REPO = "cgeosoft/cocad";
  const RELEASES = `https://github.com/${REPO}/releases/latest`;
  const CACHE_KEY = "cocad_release_v1";
  const CACHE_MS = 10 * 60 * 1000;
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const $$ = (sel) => Array.from(document.querySelectorAll(sel));

  // Downloads --------------------------------------------------------------

  const ASSETS = {
    "linux-installer": /_amd64\.deb$/,
    "linux-portable": /-linux-x86_64\.tar\.gz$/,
    "windows-installer": /-windows-x86_64\.zip$/,
    "macos-installer": /-macos-universal\.zip$/,
  };
  const LABELS = {
    linux: { name: "Linux", ext: ".deb", key: "linux-installer" },
    windows: { name: "Windows", ext: ".zip", key: "windows-installer" },
    macos: { name: "macOS", ext: ".zip", key: "macos-installer" },
  };

  function detectOS() {
    const nav = window.navigator;
    const probe = `${(nav.userAgentData && nav.userAgentData.platform) || ""} ${nav.platform || ""} ${nav.userAgent || ""}`.toLowerCase();
    if (probe.includes("win")) return "windows";
    if (probe.includes("mac")) return "macos";
    return "linux";
  }

  let activeOS = detectOS();
  let release = null;

  function render() {
    const label = LABELS[activeOS];
    const file = release && release.files[label.key];
    const button = document.getElementById("primary-download-btn");
    button.href = file ? file.url : RELEASES;
    document.getElementById("download-title").textContent = `Download for ${label.name}`;
    const parts = [label.ext];
    if (release) parts.unshift(`v${release.version}`);
    if (file) parts.push(`${Math.round(file.size / 1048576)} MB`);
    document.getElementById("download-meta").textContent = parts.join(" · ");
    $$(".os-switch").forEach((b) => b.classList.toggle("active", b.dataset.targetOs === activeOS));
    $$(".dl-row").forEach((r) => r.classList.toggle("is-detected", r.id === `card-${activeOS}`));
    $$("[data-dl]").forEach((a) => {
      const f = release && release.files[a.dataset.dl];
      a.href = f ? f.url : RELEASES;
    });
    if (!release) return;
    $$(".latest-version-text").forEach((n) => { n.textContent = `Latest: v${release.version}.`; });
    $$(".footer-version").forEach((n) => { n.textContent = `v${release.version}`; });
    // Marks that name a coming feature go away when the release has it.
    $$("[data-since]").forEach((n) => { n.hidden = compare(release.version, n.dataset.since) >= 0; });
  }

  function compare(a, b) {
    const x = a.split(".").map(Number), y = b.split(".").map(Number);
    for (let i = 0; i < 3; i++) if ((x[i] || 0) !== (y[i] || 0)) return (x[i] || 0) - (y[i] || 0);
    return 0;
  }

  function normalise(data) {
    const version = String(data.tag_name || "").replace(/^v/, "");
    if (!version || !Array.isArray(data.assets)) return null;
    const files = {};
    Object.keys(ASSETS).forEach((key) => {
      const a = data.assets.find((x) => ASSETS[key].test(x.name));
      if (a) files[key] = { url: a.browser_download_url, size: a.size };
    });
    return { version, files };
  }

  async function loadRelease() {
    try {
      const cached = JSON.parse(localStorage.getItem(CACHE_KEY) || "null");
      if (cached && Date.now() - cached.at < CACHE_MS) {
        release = cached.release;
        return render();
      }
    } catch (err) { /* Storage can be blocked. */ }
    try {
      const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: "application/vnd.github+json" } });
      if (!res.ok) return;
      release = normalise(await res.json());
      if (!release) return;
      render();
      try { localStorage.setItem(CACHE_KEY, JSON.stringify({ at: Date.now(), release })); } catch (err) { /* See above. */ }
    } catch (err) { /* The links stay on the releases page. */ }
  }

  $$(".os-switch").forEach((b) => b.addEventListener("click", () => {
    activeOS = b.dataset.targetOs;
    render();
  }));

  $$("#primary-download-btn, [data-dl]").forEach((a) => a.addEventListener("click", () => {
    const kind = a.dataset.dl || LABELS[activeOS].key;
    window.scadAnalytics.track("download_clicked", { os: kind.split("-")[0], type: kind.split("-")[1] });
  }));

  // Scroll reveal ----------------------------------------------------------

  function reveal() {
    const items = $$(".reveal");
    if (reduced || !("IntersectionObserver" in window)) {
      items.forEach((n) => n.classList.add("is-in"));
      return;
    }
    const seen = new IntersectionObserver((entries) => {
      entries.forEach((e) => {
        if (!e.isIntersecting) return;
        e.target.classList.add("is-in");
        seen.unobserve(e.target);
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.05 });
    items.forEach((n) => {
      const siblings = Array.from(n.parentElement.children).filter((c) => c.classList.contains("reveal"));
      n.style.setProperty("--delay", `${Math.min(siblings.indexOf(n), 5) * 60}ms`);
      seen.observe(n);
    });
  }

  // Screenshot tabs --------------------------------------------------------

  function tour() {
    const tabs = $$(".tour-tab");
    const ink = document.querySelector(".tour-ink");
    const img = document.querySelector("[data-tour-img]");
    const text = document.querySelector("[data-tour-text]");
    let active = tabs[0];
    const place = () => {
      ink.style.width = `${active.offsetWidth}px`;
      ink.style.transform = `translateX(${active.offsetLeft}px)`;
    };
    const select = (tab) => {
      active = tab;
      tabs.forEach((t) => t.setAttribute("aria-selected", String(t === tab)));
      place();
      text.textContent = tab.dataset.text;
      const next = new Image();
      next.src = tab.dataset.shot;
      img.classList.add("is-swap");
      next.decode().catch(() => {}).then(() => {
        img.src = tab.dataset.shot;
        img.classList.remove("is-swap");
      });
    };
    tabs.forEach((t) => t.addEventListener("click", () => select(t)));
    tabs[0].setAttribute("aria-selected", "true");
    place();
    window.addEventListener("resize", place);
    if (document.fonts && document.fonts.ready) document.fonts.ready.then(place);
  }

  // Lightbox ---------------------------------------------------------------

  function lightbox() {
    const box = document.querySelector(".lightbox");
    const img = box.querySelector("img");
    $$("[data-zoom]").forEach((b) => b.addEventListener("click", () => {
      img.src = b.dataset.zoom;
      img.alt = b.querySelector("img").alt;
      box.showModal();
    }));
    box.addEventListener("click", () => box.close());
  }

  render();
  loadRelease();
  reveal();
  tour();
  lightbox();
})();
