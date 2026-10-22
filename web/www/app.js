const PREVIEW_LIMIT = 100_000;

const $ = (id) => document.getElementById(id);
const ui = {
  dropzone: $("dropzone"),
  fileInput: $("file-input"),
  options: {
    compact: $("opt-compact"),
    keepInternal: $("opt-keep-internal"),
    legacy: $("opt-legacy"),
    raw: $("opt-raw"),
  },
  status: $("status"),
  statusText: $("status-text"),
  statusTime: $("status-time"),
  progress: $("progress"),
  error: $("error"),
  errorText: $("error-text"),
  result: $("result"),
  resultName: $("result-name"),
  resultSummary: $("result-summary"),
  downloadZip: $("download-zip"),
  downloadJson: $("download-json"),
  downloadRaw: $("download-raw"),
  copyJson: $("copy-json"),
  preview: $("preview-code"),
  previewNote: $("preview-note"),
  version: $("version"),
};

let worker = null;
let nextId = 1;
const pending = new Map();
let file = null;
let result = null;
let latestConversion = 0;

function startWorker() {
  worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
  worker.onmessage = ({ data }) => {
    const request = pending.get(data.id);
    if (!request) return;
    if (data.type === "status") {
      request.onStatus?.(data.stage);
      return;
    }
    pending.delete(data.id);
    if (data.type === "error") {
      if (data.fatal) resetWorker();
      request.reject(new Error(data.message));
    } else {
      request.resolve(data);
    }
  };
  worker.onerror = (event) => {
    event.preventDefault();
    const error = new Error(
      "Couldn't load the converter. If you are running the page locally, build it first with web/build.sh.",
    );
    for (const request of pending.values()) request.reject(error);
    pending.clear();
    resetWorker();
  };
}

function resetWorker() {
  worker?.terminate();
  worker = null;
}

function request(message, { transfer = [], onStatus } = {}) {
  if (!worker) startWorker();
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject, onStatus });
    worker.postMessage({ ...message, id }, transfer);
  });
}

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

function formatMs(ms) {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}

function plural(count, word) {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

function setBusy(busy) {
  for (const button of [ui.downloadZip, ui.downloadJson, ui.downloadRaw, ui.copyJson]) {
    button.disabled = busy;
  }
}

function showStatus(text, { done = false } = {}) {
  ui.status.hidden = false;
  ui.statusText.textContent = text;
  ui.progress.classList.toggle("done", done);
  if (done) {
    ui.progress.removeAttribute("aria-busy");
  } else {
    ui.progress.setAttribute("aria-busy", "true");
    ui.statusTime.textContent = "";
  }
}

function showError(message) {
  ui.status.hidden = true;
  ui.progress.removeAttribute("aria-busy");
  ui.error.hidden = false;
  ui.errorText.textContent = message;
}

function clearError() {
  ui.error.hidden = true;
  ui.errorText.textContent = "";
}

function readOptions() {
  return Object.fromEntries(
    Object.entries(ui.options).map(([name, input]) => [name, input.checked]),
  );
}

function baseName(name) {
  return name.replace(/\.(fig|zip)$/i, "") || "design";
}

async function runConversion() {
  if (!file) return;
  const conversion = ++latestConversion;
  clearError();
  setBusy(true);
  showStatus(`Reading ${file.name}…`);
  try {
    const bytes = await file.arrayBuffer();
    const data = await request(
      { type: "convert", bytes, options: readOptions() },
      {
        transfer: [bytes],
        onStatus: (stage) => conversion === latestConversion && showStatus(stage),
      },
    );
    if (conversion !== latestConversion) return;
    result = { ...data, name: file.name, options: readOptions() };
    renderResult();
    showStatus("Done", { done: true });
    ui.statusTime.textContent = formatMs(data.elapsedMs);
  } catch (error) {
    if (conversion !== latestConversion) return;
    result = null;
    ui.result.hidden = true;
    showError(error.message);
  } finally {
    if (conversion === latestConversion) setBusy(false);
  }
}

function renderResult() {
  const jsonBytes = new Blob([result.json]).size;
  ui.resultName.textContent = result.name;
  const parts = [`canvas.json ${formatBytes(jsonBytes)}`];
  if (result.files.length) {
    parts.push(plural(result.imageCount, "image"));
    const others = result.files.length - result.imageCount;
    if (others) parts.push(plural(others, "other file"));
  } else {
    parts.push("no images (bare canvas.fig)");
  }
  parts.push(`converted in ${formatMs(result.elapsedMs)}`);
  ui.resultSummary.textContent = parts.join(" · ");

  const truncated = result.json.length > PREVIEW_LIMIT;
  ui.preview.textContent = truncated
    ? `${result.json.slice(0, PREVIEW_LIMIT)}\n…`
    : result.json;
  ui.previewNote.textContent = truncated
    ? `Showing the first ${formatBytes(PREVIEW_LIMIT)} of ${formatBytes(jsonBytes)}. Download or copy for the full JSON.`
    : "";
  ui.downloadRaw.hidden = result.rawJson === null;
  ui.result.hidden = false;
}

function download(blob, name) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

async function downloadZip() {
  if (!result) return;
  clearError();
  setBusy(true);
  try {
    const data = await request({ type: "zip" }, { onStatus: (stage) => showStatus(stage) });
    download(new Blob([data.zip], { type: "application/zip" }), `${baseName(result.name)}.zip`);
    showStatus(`ZIP ready (${formatBytes(data.zip.byteLength)})`, { done: true });
  } catch (error) {
    showError(error.message);
  } finally {
    setBusy(false);
  }
}

async function copyJson() {
  if (!result) return;
  const label = ui.copyJson.textContent;
  try {
    await navigator.clipboard.writeText(result.json);
    ui.copyJson.textContent = "Copied!";
  } catch {
    ui.copyJson.textContent = "Copy failed";
  }
  setTimeout(() => (ui.copyJson.textContent = label), 1600);
}

function selectFile(selected) {
  if (!selected) return;
  file = selected;
  runConversion();
}

ui.fileInput.addEventListener("change", () => {
  selectFile(ui.fileInput.files[0]);
  ui.fileInput.value = "";
});

for (const type of ["dragenter", "dragover"]) {
  ui.dropzone.addEventListener(type, (event) => {
    event.preventDefault();
    ui.dropzone.classList.add("dragover");
  });
}
for (const type of ["dragleave", "drop"]) {
  ui.dropzone.addEventListener(type, (event) => {
    event.preventDefault();
    if (type === "dragleave" && ui.dropzone.contains(event.relatedTarget)) return;
    ui.dropzone.classList.remove("dragover");
  });
}
ui.dropzone.addEventListener("drop", (event) => selectFile(event.dataTransfer.files[0]));
window.addEventListener("dragover", (event) => event.preventDefault());
window.addEventListener("drop", (event) => event.preventDefault());

for (const input of Object.values(ui.options)) {
  input.addEventListener("change", runConversion);
}

ui.downloadZip.addEventListener("click", downloadZip);
ui.downloadJson.addEventListener("click", () =>
  download(new Blob([result.json], { type: "application/json" }), "canvas.json"),
);
ui.downloadRaw.addEventListener("click", () =>
  download(new Blob([result.rawJson], { type: "application/json" }), "canvas.raw.json"),
);
ui.copyJson.addEventListener("click", copyJson);

request({ type: "version" })
  .then(({ version }) => (ui.version.textContent = `v${version}`))
  .catch(() => {});
