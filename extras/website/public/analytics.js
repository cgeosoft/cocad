/**
 * Consent modal and PostHog analytics.
 * PostHog loads only after the visitor accepts. Before that, the page makes
 * no request to it. deploy.sh writes the project key. With no key, this file
 * does nothing and the modal does not show.
 */
(function () {
  "use strict";

  const KEY = "YOUR_POSTHOG_PROJECT_API_KEY";
  const SCRIPT = "https://eu-assets.i.posthog.com/static/array.js";
  const CONSENT_KEY = "cocad_consent_v1";
  const CONSENT_MS = 365 * 24 * 60 * 60 * 1000;
  const enabled = /^phc_/.test(KEY);

  let choice = null;
  let dialog = null;
  let loading = null;

  function read() {
    try {
      const saved = JSON.parse(localStorage.getItem(CONSENT_KEY) || "null");
      if (saved && Date.now() - saved.at < CONSENT_MS) return saved.value;
    } catch (err) { /* Storage can be blocked. */ }
    return null;
  }

  function save(value) {
    choice = value;
    try {
      localStorage.setItem(CONSENT_KEY, JSON.stringify({ value, at: Date.now() }));
    } catch (err) { /* The choice stays for this page view. */ }
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
          capture_pageview: true,
          capture_pageleave: false,
          disable_session_recording: true,
          disable_surveys: true,
          advanced_disable_feature_flags: true,
          disable_external_dependency_loading: true,
          respect_dnt: true,
        });
        window.posthog.register({ site: "cocad" });
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
      load().then(() => window.posthog.opt_in_capturing()).catch(() => {});
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

  function buildDialog() {
    dialog = document.createElement("dialog");
    dialog.className = "consent";
    dialog.setAttribute("aria-labelledby", "consent-title");
    dialog.innerHTML =
      '<p class="label"><span class="dot"></span>Cookies and analytics</p>' +
      '<h2 id="consent-title">Allow analytics?</h2>' +
      "<p>With your OK, PostHog counts page views, download clicks and demo use, so I can see what to improve. " +
      "It keeps a random ID in your browser storage. It sets no cookies and records no sessions. The site works the same if you decline.</p>" +
      '<p class="consent-state mono" data-consent-status></p>' +
      '<div class="consent-actions">' +
      '<button type="button" class="btn" data-choice="declined">Decline</button>' +
      '<button type="button" class="btn" data-choice="accepted">Accept</button>' +
      "</div>" +
      '<a class="consent-link" href="/privacy/">Privacy policy</a>';
    dialog.addEventListener("click", (e) => {
      const button = e.target.closest("[data-choice]");
      if (!button) return;
      apply(button.dataset.choice);
      dialog.close();
    });
    document.body.append(dialog);
  }

  function open() {
    if (!dialog) buildDialog();
    syncStatus();
    if (!dialog.open) dialog.showModal();
  }

  function track(name, props) {
    if (choice !== "accepted" || !loading) return;
    loading.then(() => window.posthog.capture(name, props || {})).catch(() => {});
  }

  window.scadAnalytics = { track };

  document.querySelectorAll("[data-cookie-settings]").forEach((b) => {
    b.hidden = !enabled;
    b.addEventListener("click", open);
  });
  if (!enabled) return;

  choice = read();
  syncStatus();
  if (choice === "accepted") load().catch(() => {});
  // The modal does not cover the policy that it links to.
  else if (!choice && !document.body.hasAttribute("data-no-consent-prompt")) setTimeout(open, 1200);
})();
