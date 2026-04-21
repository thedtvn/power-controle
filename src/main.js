const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

const SCHEME_LABELS = {
  balanced: "Balanced",
  high_performance: "High Performance",
  power_saver: "Power Saver",
};

// --- DOM refs ---
const elScheme = document.getElementById("current-scheme");
const elSource = document.getElementById("power-source");
const elAutostart = document.getElementById("autostart");
const elAutoSwitch = document.getElementById("auto-switch");
const elAutoOpts = document.getElementById("auto-opts");
const elOnAc = document.getElementById("on-ac");
const elOnBattery = document.getElementById("on-battery");
const elSaveBtn = document.getElementById("save-btn");
const elToast = document.getElementById("toast");

// --- Custom titlebar close ---
document.getElementById("titlebar-close").addEventListener("click", () => {
  getCurrentWindow().close();
});

// --- Helpers ---
function formatSource(source, pct) {
  if (source === "ac") return "AC Power";
  if (source === "battery") return pct <= 100 ? `Battery (${pct}%)` : "Battery";
  return "Unknown";
}

function renderStatus(status) {
  elScheme.textContent = SCHEME_LABELS[status.active_scheme] || status.active_scheme;
  elSource.textContent = formatSource(status.power_source, status.battery_percent);
}

function syncAutoOpts() {
  elAutoOpts.classList.toggle("disabled", !elAutoSwitch.checked);
}

let _toastTimer;
function toast(msg) {
  elToast.textContent = msg;
  elToast.classList.remove("hidden");
  clearTimeout(_toastTimer);
  _toastTimer = setTimeout(() => elToast.classList.add("hidden"), 2000);
}

// --- Init ---
async function init() {
  try {
    const status = await invoke("cmd_get_status");
    renderStatus(status);
  } catch (e) {
    console.error("status:", e);
  }

  try {
    const cfg = await invoke("cmd_get_config");
    elAutoSwitch.checked = cfg.auto_switch;
    elOnAc.value = cfg.on_ac;
    elOnBattery.value = cfg.on_battery;
    syncAutoOpts();
  } catch (e) {
    console.error("config:", e);
  }

  // Autostart state (independent of config file — managed by OS registry)
  try {
    elAutostart.checked = await invoke("cmd_get_autostart");
  } catch (e) {
    console.error("autostart:", e);
  }

  // Live updates from backend (power source change, tray scheme change)
  listen("status-changed", (ev) => renderStatus(ev.payload));
}

// --- Events ---

// Autostart toggles immediately (OS-level, no "Save" needed)
elAutostart.addEventListener("change", async () => {
  try {
    await invoke("cmd_set_autostart", { enabled: elAutostart.checked });
    toast(elAutostart.checked ? "Autostart enabled" : "Autostart disabled");
  } catch (e) {
    elAutostart.checked = !elAutostart.checked; // revert on failure
    toast("Error: " + e);
  }
});

elAutoSwitch.addEventListener("change", syncAutoOpts);

elSaveBtn.addEventListener("click", async () => {
  try {
    await invoke("cmd_save_config", {
      config: {
        auto_switch: elAutoSwitch.checked,
        on_ac: elOnAc.value,
        on_battery: elOnBattery.value,
      },
    });
    toast("Settings saved");
  } catch (e) {
    toast("Error: " + e);
  }
});

init();
