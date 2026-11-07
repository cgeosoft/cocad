/**
 * Consent box and PostHog analytics.
 * PostHog loads only after the visitor accepts. Before that, the page makes
 * no request to it. deploy.sh writes the project key. With no key, this file
 * does nothing and the box does not show.
 *
 * Add an event or a property only with a change to public/privacy/index.html.
 * Raise NOTICE_VERSION, as in src/telemetry/consent.rs, when the data, the
 * purpose or the recipient changes.
 */
(function () {
  "use strict";

  const KEY = "YOUR_POSTHOG_PROJECT_API_KEY";
  const SCRIPT = "https://eu-assets.i.posthog.com/static/array.js";
  const CONSENT_KEY = "cocad_consent_v2";
  const OLD_KEY = "cocad_consent_v1";
  const CONSENT_MS = 365 * 24 * 60 * 60 * 1000;
  const NOTICE_VERSION = 1;
  const EVENTS = ["page_view", "download_clicked", "demo_prompt"];
  const PROPERTIES = [
    "token", "distinct_id", "$lib", "$lib_version", "$insert_id", "$time", "$process_person_profile",
    "$geoip_disable", "site", "consent_version", "os", "type", "model",
  ];
  const enabled = /^phc_/.test(KEY);

  let choice = null;
  let box = null;
  let loading = null;
  let source = "setup";

  /** A choice for an older notice does not count, so the box shows again. */
  function read() {
    try {
      localStorage.removeItem(OLD_KEY);
      const saved = JSON.parse(localStorage.getItem(CONSENT_KEY) || "null");
      const fresh = saved && Date.now() - Date.parse(saved.at) < CONSENT_MS;
      if (fresh && saved.version === NOTICE_VERSION) return saved.value;
    } catch (err) { /* Storage can be blocked. */ }
    return null;
  }

  function save(value) {
    choice = value;
    try {
      const record = { value, at: new Date().toISOString(), source, version: NOTICE_VERSION };
      localStorage.setItem(CONSENT_KEY, JSON.stringify(record));
    } catch (err) { /* The choice stays for this page view. */ }
  }

  /** The last step before a send. Events and fields that are not listed go. */
  function allow(event) {
    if (!event || !EVENTS.includes(event.event)) return null;
    const properties = {};
    PROPERTIES.forEach((key) => {
      if (key in event.properties) properties[key] = event.properties[key];
    });
    const { $set, $set_once, ...rest } = event;
    return { ...rest, properties };
  }

  function clearStored() {
    try {
      Object.keys(localStorage).filter((k) => k.startsWith("ph_") || k.startsWith("__ph_")).forEach((k) => localStorage.removeItem(k));
    } catch (err) { /* See above. */ }
  }

  function load() {
    if (loading) return loading;
    loading = new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = SCRIPT;
      script.async = true;
      script.onload = () => {
        window.posthog.init(KEY, {
          api_host: "https://eu.i.posthog.com",
          persistence: "localStorage",
          person_profiles: "identified_only",
          autocapture: false,
          capture_pageview: false,
          capture_pageleave: false,
          rageclick: false,
          capture_dead_clicks: false,
          capture_heatmaps: false,
          capture_exceptions: false,
          capture_performance: false,
          disable_session_recording: true,
          disable_surveys: true,
          advanced_disable_feature_flags: true,
          disable_external_dependency_loading: true,
          respect_dnt: true,
          before_send: allow,
        });
        window.posthog.register({ site: "cocad", consent_version: NOTICE_VERSION, $geoip_disable: true });
        resolve();
      };
      script.onerror = reject;
      document.head.append(script);
    });
    loading.catch(() => { loading = null; });
    return loading;
  }

  function apply(value) {
    save(value);
    if (value === "accepted") {
      load().then(() => {
        window.posthog.opt_in_capturing();
        track("page_view");
      }).catch(() => {});
    } else {
      if (window.posthog && window.posthog.opt_out_capturing) window.posthog.opt_out_capturing();
      clearStored();
    }
    syncStatus();
  }

  function syncStatus() {
    const text = choice === "accepted" ? "Analytics are on." : choice === "declined" ? "Analytics are off." : "You have not made a choice.";
    document.querySelectorAll("[data-consent-status]").forEach((n) => { n.textContent = text; });
  }

  function buildBox() {
    box = document.createElement("div");
    box.className = "consent";
    box.setAttribute("role", "dialog");
    box.setAttribute("aria-labelledby", "consent-title");
    box.innerHTML =
      '<div class="consent-title" id="consent-title">Analytics?</div>' +
      "<p>With your OK, PostHog counts page views, download clicks and demo use. " +
      "It keeps a random ID in your browser storage. It sets no cookies and records no sessions. " +
      '<a href="/privacy/">Privacy Notice</a>.</p>' +
      '<p class="consent-state mono" data-consent-status hidden></p>' +
      '<div class="consent-actions">' +
      '<button type="button" class="btn" data-choice="declined">Decline</button>' +
      '<button type="button" class="btn" data-choice="accepted">Accept</button>' +
      "</div>";
    box.addEventListener("click", (e) => {
      const button = e.target.closest("[data-choice]");
      if (!button) return;
      apply(button.dataset.choice);
      box.classList.remove("visible");
    });
    document.body.append(box);
  }

  function open() {
    if (!box) buildBox();
    syncStatus();
    // The state line is for a visitor who opens the box again from the footer.
    box.querySelector("[data-consent-status]").hidden = !choice;
    requestAnimationFrame(() => box.classList.add("visible"));
  }

  function track(name, props) {
    if (choice !== "accepted" || !loading) return;
    loading.then(() => window.posthog.capture(name, props || {})).catch(() => {});
  }

  window.scadAnalytics = { track };

  document.querySelectorAll("[data-cookie-settings]").forEach((b) => {
    b.hidden = !enabled;
    b.addEventListener("click", () => {
      source = "settings";
      open();
    });
  });
  if (!enabled) return;

  choice = read();
  syncStatus();
  if (choice === "accepted") load().then(() => track("page_view")).catch(() => {});
  // The box does not cover the policy that it links to.
  else if (!choice && !document.body.hasAttribute("data-no-consent-prompt")) setTimeout(open, 1200);
})();
