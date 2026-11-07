// Consent tests for public/analytics.js. A fake browser runs the file, so no
// test reaches PostHog.
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const SOURCE = readFileSync(new URL("../public/analytics.js", import.meta.url), "utf8");

function browser({ key = "phc_test", stored = {} } = {}) {
  const storage = new Map(Object.entries(stored));
  const scripts = [];
  const boxes = [];
  const node = () => ({
    classList: { add() {}, remove() {} },
    handlers: {},
    hidden: false,
    setAttribute() {},
    addEventListener(name, fn) { this.handlers[name] = fn; },
    querySelector: () => ({ hidden: false }),
    append() {},
  });
  const footer = node();
  const localStorage = {
    getItem: (k) => (storage.has(k) ? storage.get(k) : null),
    setItem: (k, v) => storage.set(k, String(v)),
    removeItem: (k) => storage.delete(k),
  };
  // Object.keys(localStorage) lists the stored keys, as in a browser.
  const proxy = new Proxy(localStorage, {
    ownKeys: () => [...storage.keys()],
    getOwnPropertyDescriptor: () => ({ enumerable: true, configurable: true }),
  });
  const document = {
    head: { append: (s) => scripts.push(s) },
    body: { append() {}, hasAttribute: () => false },
    createElement(tag) {
      const n = node();
      if (tag === "div") boxes.push(n);
      return n;
    },
    querySelectorAll: (q) => (q === "[data-cookie-settings]" ? [footer] : []),
  };
  const window = {};
  const timers = [];
  const run = new Function("window", "document", "localStorage", "setTimeout", "requestAnimationFrame",
    SOURCE.replace("YOUR_POSTHOG_PROJECT_API_KEY", key));
  run(window, document, proxy, (fn) => timers.push(fn), () => {});
  timers.forEach((fn) => fn());

  const captured = [];
  const posthog = {
    config: null,
    registered: {},
    optedOut: false,
    init(_, config) { this.config = config; },
    register(p) { Object.assign(this.registered, p); },
    opt_in_capturing() {},
    opt_out_capturing() { this.optedOut = true; },
    capture(name, props) { captured.push({ name, props }); },
  };
  return {
    storage, scripts, boxes, footer, window, posthog, captured,
    choose(value) {
      boxes[0].handlers.click({ target: { closest: () => ({ dataset: { choice: value } }) } });
    },
    async loadSdk() {
      window.posthog = posthog;
      scripts[0].onload();
      await new Promise((r) => setImmediate(r));
    },
  };
}

test("no key: no box and no script", () => {
  const b = browser({ key: "YOUR_POSTHOG_PROJECT_API_KEY" });
  expect(b.boxes.length).toBe(0);
  expect(b.scripts.length).toBe(0);
  expect(b.footer.hidden).toBe(true);
});

test("no PostHog request before consent", () => {
  const b = browser();
  expect(b.boxes.length).toBe(1);
  expect(b.scripts.length).toBe(0);
});

test("decline makes no request and records the choice", () => {
  const b = browser();
  b.choose("declined");
  expect(b.scripts.length).toBe(0);
  const record = JSON.parse(b.storage.get("cocad_consent_v2"));
  expect(record.value).toBe("declined");
  expect(record.version).toBe(1);
  expect(record.source).toBe("setup");
  expect(Date.parse(record.at)).toBeGreaterThan(0);
});

test("the two buttons have the same style", () => {
  const b = browser();
  const html = b.boxes[0].innerHTML;
  const buttons = html.match(/<button[^>]*class="[^"]*"/g);
  expect(buttons.length).toBe(2);
  expect(new Set(buttons.map((t) => t.match(/class="([^"]*)"/)[1])).size).toBe(1);
});

test("accept loads PostHog with no autocapture and sends a named page view", async () => {
  const b = browser();
  b.choose("accepted");
  expect(b.scripts[0].src).toBe("https://eu-assets.i.posthog.com/static/array.js");
  await b.loadSdk();
  const c = b.posthog.config;
  expect(c.autocapture).toBe(false);
  expect(c.capture_pageview).toBe(false);
  expect(c.disable_session_recording).toBe(true);
  expect(c.persistence).toBe("localStorage");
  expect(b.posthog.registered.consent_version).toBe(1);
  expect(b.captured.map((e) => e.name)).toEqual(["page_view"]);
});

test("the send hook keeps only listed events and properties", async () => {
  const b = browser();
  b.choose("accepted");
  await b.loadSdk();
  const allow = b.posthog.config.before_send;
  expect(allow({ event: "$opt_in", properties: {} })).toBe(null);
  expect(allow({ event: "$pageview", properties: {} })).toBe(null);
  const sent = allow({
    event: "download_clicked",
    uuid: "u",
    $set: { $current_url: "https://x/?secret" },
    $set_once: { $initial_referrer: "https://y/" },
    properties: {
      token: "t", distinct_id: "d", os: "linux", type: "deb", site: "cocad", consent_version: 1,
      $current_url: "https://x/?secret", $referrer: "https://y/", $pathname: "/secret", $browser: "b",
      $screen_height: 1, $session_id: "s", other: "z",
    },
  });
  expect(Object.keys(sent.properties).sort()).toEqual(["consent_version", "distinct_id", "os", "site", "token", "type"]);
  expect(sent.$set).toBeUndefined();
  expect(sent.$set_once).toBeUndefined();
  expect(sent.uuid).toBe("u");
});

test("decline after accept stops PostHog and removes its ID", async () => {
  const b = browser();
  b.choose("accepted");
  await b.loadSdk();
  b.storage.set("ph_phc_test_posthog", "id");
  b.storage.set("__ph_opt_in_out_phc_test", "1");
  b.footer.handlers.click();
  b.choose("declined");
  expect(b.posthog.optedOut).toBe(true);
  expect([...b.storage.keys()].filter((k) => k.includes("ph_"))).toEqual([]);
  expect(JSON.parse(b.storage.get("cocad_consent_v2")).source).toBe("settings");
});

test("a saved accept starts PostHog on the next visit", async () => {
  const record = { value: "accepted", at: new Date().toISOString(), source: "setup", version: 1 };
  const b = browser({ stored: { cocad_consent_v2: JSON.stringify(record) } });
  expect(b.boxes.length).toBe(0);
  await b.loadSdk();
  expect(b.captured.map((e) => e.name)).toEqual(["page_view"]);
});

test("an old notice or an old key asks again", () => {
  const older = { value: "accepted", at: new Date().toISOString(), source: "setup", version: 0 };
  const b = browser({ stored: { cocad_consent_v2: JSON.stringify(older) } });
  expect(b.scripts.length).toBe(0);
  expect(b.boxes.length).toBe(1);
  const v1 = browser({ stored: { cocad_consent_v1: JSON.stringify({ value: "accepted", at: Date.now() }) } });
  expect(v1.scripts.length).toBe(0);
  expect(v1.boxes.length).toBe(1);
  expect(v1.storage.has("cocad_consent_v1")).toBe(false);
});

test("a choice older than 12 months asks again", () => {
  const at = new Date(Date.now() - 366 * 24 * 60 * 60 * 1000).toISOString();
  const record = { value: "accepted", at, source: "setup", version: 1 };
  const b = browser({ stored: { cocad_consent_v2: JSON.stringify(record) } });
  expect(b.scripts.length).toBe(0);
  expect(b.boxes.length).toBe(1);
});
