/**
 * The two live views of the home page: the hero loop and the agent loop demo.
 * Meshes and numbers come from assets/mesh/models.json (real CLI output).
 * The agent text in the demo is scripted. The tool names are the real MCP tools.
 */
(function () {
  "use strict";

  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const sleep = (ms) => new Promise((r) => setTimeout(r, reduced ? 0 : ms));
  const $ = (sel, root = document) => root.querySelector(sel);
  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text !== undefined) n.textContent = text;
    return n;
  };
  const fmt = (n) => n.toLocaleString("en-US");

  let models = null;

  function paramsOf(slug) {
    return models[slug].groups.flatMap((g) => g.params);
  }

  function variantKey(slug, state) {
    return paramsOf(slug).map((p) => {
      const v = state[p.name];
      return typeof v === "boolean" ? String(Number(v)) : String(v);
    }).join("-");
  }

  function meshUrl(slug, key) {
    return `assets/mesh/${slug}/${key}.bin`;
  }

  // Hero -------------------------------------------------------------------

  const HERO_STEPS = [
    { sides: 5, twist: 240 },
    { sides: 3, twist: 360 },
    { sides: 7, twist: 120 },
    { sides: 5, twist: 0 },
  ];

  async function hero() {
    const root = $("[data-hero]");
    const canvas = $(".hero-canvas", root);
    const wide = () => window.innerWidth > 900;
    const viewer = new ScadView.Viewer(canvas, { autoRotate: !reduced, shift: wide() ? 0.42 : 0, margin: 1.3 });
    window.addEventListener("resize", () => { viewer.shift = wide() ? 0.42 : 0; viewer.wake(); });

    const call = $("[data-hero-call]", root);
    const result = $("[data-hero-result]", root);
    const stages = [...root.querySelectorAll("[data-hero-stage]")];
    const stage = (n) => stages.forEach((s, i) => {
      s.classList.toggle("is-on", i === n);
      s.classList.toggle("is-done", i < n);
    });

    const show = async (step, opts) => {
      const state = { ...models.vase.defaults, ...step };
      const key = variantKey("vase", state);
      const mesh = await ScadView.loadMesh(meshUrl("vase", key));
      const data = models.vase.variants[key];
      call.textContent = `set_params ${JSON.stringify(step)}`;
      viewer.setMode("solid");
      viewer.setMesh(mesh, opts);
      return data;
    };

    const first = await show(HERO_STEPS[2], { reveal: !reduced, snap: true });
    result.textContent = `${first.seconds.toFixed(2)} s · ${fmt(first.triangles)} triangles`;
    root.classList.add("is-ready");
    stage(1);
    if (reduced) return;

    let i = 0;
    for (;;) {
      await sleep(2600);
      while (!viewer.visible || document.hidden) await sleep(500);
      const data = viewer.mesh && models.vase.variants[variantKey("vase", { ...models.vase.defaults, ...HERO_STEPS[(i + 2) % 4] })];
      stage(2);
      result.textContent = `slice: ${fmt(data.layers)} layers · ${data.grams.toFixed(0)} g`;
      viewer.setMode("slice");
      viewer.setLayer(0);
      await viewer.sweep(3000);
      await sleep(900);
      viewer.setMode("solid");
      stage(0);
      const step = HERO_STEPS[(i + 3) % 4];
      i++;
      const next = await show(step);
      await sleep(350);
      stage(1);
      result.textContent = `${next.seconds.toFixed(2)} s · ${fmt(next.triangles)} triangles`;
    }
  }

  // Agent loop demo --------------------------------------------------------

  const PROMPTS = [
    { model: "vase", text: "Give the vase five sides and more twist.", set: { sides: 5, twist: 240 } },
    { model: "vase", text: "Make it straight, with three sides.", set: { sides: 3, twist: 0 } },
    { model: "gear", text: "I need a 40 tooth gear with a taller hub.", set: { teeth: 40, hub_height: 12 } },
    { model: "gear", text: "Remove the hub.", set: { hub: false } },
    { model: "box", text: "Three compartments and square corners.", set: { compartments: 3, corners: "square" } },
  ];

  const state = { slug: "vase", values: {}, viewer: null, run: 0 };

  function setStep(n) {
    document.querySelectorAll("[data-loop-stage]").forEach((s, i) => {
      s.classList.toggle("is-on", i === n);
      s.classList.toggle("is-done", i < n);
    });
  }

  function current() {
    return models[state.slug].variants[variantKey(state.slug, state.values[state.slug])];
  }

  function setMode(mode) {
    const v = state.viewer;
    v.stopSweep();
    v.setMode(mode);
    document.querySelectorAll("[data-mode]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.mode === mode)));
    $("[data-layer-bar]").hidden = mode !== "slice";
    if (mode === "slice") v.setLayer(Number($("[data-layer]").value) / 1000);
  }

  function showStats() {
    const d = current();
    const set = (key, text) => { $(`[data-stat="${key}"]`).textContent = text; };
    set("file", models[state.slug].file);
    set("time", `Ready · ${d.seconds.toFixed(2)} s`);
    set("size", d.size.map((n) => n.toFixed(1)).join(" × ") + " mm");
    set("tris", `${fmt(d.triangles)} triangles`);
    set("volume", `${d.volume.toFixed(2)} cm³`);
    set("edges", `${d.open_edges} open edges`);
    updateLayerLabel();
  }

  function updateLayerLabel() {
    const d = current();
    const layer = Math.max(1, Math.round((Number($("[data-layer]").value) / 1000) * d.layers));
    $("[data-layer-label]").textContent = `Layer ${fmt(layer)} / ${fmt(d.layers)}`;
  }

  async function applyMesh(reveal) {
    const key = variantKey(state.slug, state.values[state.slug]);
    const mesh = await ScadView.loadMesh(meshUrl(state.slug, key));
    state.viewer.setMesh(mesh, { reveal: reveal && !reduced });
    showStats();
    syncControls();
  }

  // Customizer tab ---------------------------------------------------------

  function buildCustomizer() {
    const host = $("[data-customizer]");
    host.textContent = "";
    const tabs = el("div", "model-tabs");
    Object.keys(models).forEach((slug) => {
      const b = el("button", "chip", models[slug].title);
      b.type = "button";
      b.setAttribute("aria-pressed", String(slug === state.slug));
      b.addEventListener("click", () => {
        state.run++;
        state.slug = slug;
        buildCustomizer();
        setMode("solid");
        setStep(1);
        applyMesh(true);
      });
      tabs.append(b);
    });
    host.append(tabs);
    const values = state.values[state.slug];
    models[state.slug].groups.forEach((group) => {
      const box = el("div", "param-group");
      box.append(el("p", "param-head", group.name));
      group.params.forEach((p) => box.append(buildParam(p, values)));
      host.append(box);
    });
    const total = Object.keys(models[state.slug].variants).length;
    host.append(el("p", "param-note mono", `This page holds ${total} prebuilt meshes of ${models[state.slug].file}. The app renders any value.`));
    syncControls();
  }

  function buildParam(p, values) {
    const row = el("div", "param");
    row.dataset.param = p.name;
    const top = el("div", "param-top");
    top.append(el("span", "param-dot"), el("label", "param-name", p.label));
    const id = `param-${p.name}`;
    $("label", top).htmlFor = id;
    const change = (value) => {
      state.run++;
      values[p.name] = value;
      setMode("solid");
      setStep(1);
      applyMesh(true);
    };
    if (p.type === "slider") {
      const out = el("output", "param-value mono");
      const input = el("input");
      input.type = "range"; input.id = id; input.min = 0; input.max = p.values.length - 1; input.step = 1;
      input.addEventListener("input", () => change(p.values[Number(input.value)]));
      top.append(out);
      row.append(top, input);
      row.sync = () => {
        const i = p.values.indexOf(values[p.name]);
        input.value = i;
        out.textContent = values[p.name];
        input.style.setProperty("--fill", `${(i / (p.values.length - 1)) * 100}%`);
        input.disabled = p.name === "hub_height" && values.hub === false;
      };
    } else if (p.type === "switch") {
      const input = el("input", "switch");
      input.type = "checkbox"; input.id = id; input.setAttribute("role", "switch");
      input.addEventListener("change", () => change(input.checked));
      top.append(input);
      row.append(top);
      row.sync = () => { input.checked = values[p.name]; };
    } else {
      const select = el("select", "param-select mono");
      select.id = id;
      p.values.forEach((v) => {
        const o = el("option", "", String(v));
        o.value = String(v);
        select.append(o);
      });
      select.addEventListener("change", () => {
        change(typeof p.values[0] === "number" ? Number(select.value) : select.value);
      });
      top.append(select);
      row.append(top);
      row.sync = () => { select.value = String(values[p.name]); };
    }
    row.dataset.default = String(models[state.slug].defaults[p.name]);
    return row;
  }

  function syncControls() {
    document.querySelectorAll("[data-customizer] .param").forEach((row) => {
      if (row.sync) row.sync();
      const value = state.values[state.slug][row.dataset.param];
      row.classList.toggle("is-edited", String(value) !== row.dataset.default);
    });
  }

  // Agent tab --------------------------------------------------------------

  function addLine(log, node) {
    log.append(node);
    log.scrollTo({ top: log.scrollHeight, behavior: reduced ? "auto" : "smooth" });
  }

  function toolRow(name, args) {
    const row = el("div", "tool-row is-busy");
    const head = el("div", "tool-head");
    head.append(el("span", "tool-name mono", name));
    if (args) head.append(el("code", "tool-args", args));
    row.append(head);
    return row;
  }

  function finish(row, text) {
    row.classList.remove("is-busy");
    row.append(el("div", "tool-out mono", text));
  }

  async function runPrompt(prompt, button) {
    const id = ++state.run;
    const alive = () => id === state.run;
    const log = $("[data-log]");
    document.querySelectorAll("[data-prompt]").forEach((b) => b.setAttribute("aria-pressed", String(b === button)));
    log.textContent = "";
    selectTab("agent");
    setMode("solid");
    setStep(0);
    addLine(log, el("p", "msg-user", prompt.text));
    await sleep(500);
    if (!alive()) return;

    if (prompt.model !== state.slug) {
      const open = toolRow("open_model", `{"path":"${models[prompt.model].file}"}`);
      addLine(log, open);
      await sleep(350);
      if (!alive()) return;
      state.slug = prompt.model;
      state.values[prompt.model] = { ...models[prompt.model].defaults };
      buildCustomizer();
      finish(open, "opened");
    }

    const params = toolRow("set_params", JSON.stringify({ values: prompt.set }));
    addLine(log, params);
    await sleep(350);
    if (!alive()) return;
    Object.assign(state.values[state.slug], prompt.set);
    setStep(1);
    await applyMesh(true);
    const d = current();
    finish(params, `rendered in ${d.seconds.toFixed(2)} s, ${fmt(d.triangles)} triangles`);
    await sleep(900);
    if (!alive()) return;

    setStep(2);
    const slice = toolRow("slice", null);
    addLine(log, slice);
    $("[data-layer]").value = 0;
    setMode("slice");
    state.viewer.setLayer(0);
    await state.viewer.sweep(reduced ? 1 : 2200);
    if (!alive()) return;
    $("[data-layer]").value = 1000;
    updateLayerLabel();
    finish(slice, `${fmt(d.layers)} layers, ${d.filament_m} m, ${d.grams} g, ${d.print_time}`);
    await sleep(500);
    if (!alive()) return;

    setStep(3);
    const edges = d.open_edges === 0 ? "No open edges." : `${d.open_edges} open edges.`;
    addLine(log, el("p", "msg-agent", `${edges} ${d.size.map((n) => n.toFixed(0)).join(" × ")} mm, ${d.print_time} at 0.2 mm. Ready to export or render a photo.`));
  }

  function selectTab(name) {
    document.querySelectorAll("[data-tab]").forEach((t) => {
      const on = t.dataset.tab === name;
      t.setAttribute("aria-selected", String(on));
    });
    document.querySelectorAll("[data-tabpanel]").forEach((p) => { p.hidden = p.dataset.tabpanel !== name; });
  }

  async function loop() {
    const root = $("[data-loop]");
    const viewer = new ScadView.Viewer($("canvas", root), { zoom: true });
    state.viewer = viewer;
    Object.keys(models).forEach((slug) => { state.values[slug] = { ...models[slug].defaults }; });

    const prompts = $("[data-prompts]");
    PROMPTS.forEach((p) => {
      const b = el("button", "prompt", p.text);
      b.type = "button";
      b.dataset.prompt = "";
      b.setAttribute("aria-pressed", "false");
      b.addEventListener("click", () => runPrompt(p, b));
      prompts.append(b);
    });

    document.querySelectorAll("[data-tab]").forEach((t) => t.addEventListener("click", () => selectTab(t.dataset.tab)));
    document.querySelectorAll("[data-view]").forEach((b) => b.addEventListener("click", () => viewer.setView(b.dataset.view)));
    document.querySelectorAll("[data-mode]").forEach((b) => b.addEventListener("click", () => {
      state.run++;
      setMode(b.dataset.mode);
    }));
    const layer = $("[data-layer]");
    layer.addEventListener("input", () => {
      state.viewer.stopSweep();
      viewer.setLayer(Number(layer.value) / 1000);
      updateLayerLabel();
    });
    viewer.onLayer = (f) => {
      layer.value = Math.round(f * 1000);
      updateLayerLabel();
    };

    buildCustomizer();
    await applyMesh(false);
    viewer.dist = viewer.distGoal;
    viewer.center = viewer.centerGoal.slice();
    root.classList.add("is-ready");

    // The first prompt runs one time, when the demo comes into view.
    new IntersectionObserver(([e], obs) => {
      if (!e.isIntersecting) return;
      obs.disconnect();
      if (state.run === 0 && !reduced) runPrompt(PROMPTS[0], prompts.firstChild);
    }, { threshold: 0.45 }).observe(root);
  }

  async function start() {
    try {
      const res = await fetch("assets/mesh/models.json");
      if (!res.ok) throw new Error(String(res.status));
      models = await res.json();
      await Promise.all([hero(), loop()]);
    } catch (err) {
      console.warn("Live views are off:", err);
      document.documentElement.classList.add("no-gl");
    }
  }

  start();
})();
