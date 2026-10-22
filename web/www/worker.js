import init, { convert, Options, version } from "./pkg/figma2json_web.js";

const ready = init();
let current = null;

function post(message, transfer = []) {
  self.postMessage(message, transfer);
}

function describe(error) {
  if (error instanceof WebAssembly.RuntimeError) {
    return {
      message:
        "The converter crashed. The file may be too large for the browser " +
        "(WebAssembly is limited to 4 GB of memory); try the command-line tool instead.",
      fatal: true,
    };
  }
  return { message: error?.message ?? String(error), fatal: false };
}

async function handleConvert({ id, bytes, options }) {
  post({ type: "status", id, stage: "Loading converter…" });
  await ready;
  if (current) {
    current.free();
    current = null;
  }

  post({ type: "status", id, stage: "Converting…" });
  const opts = new Options();
  opts.compact = options.compact;
  opts.legacy = options.legacy;
  opts.keepInternal = options.keepInternal;
  opts.raw = options.raw;
  const started = performance.now();
  try {
    current = convert(new Uint8Array(bytes), opts);
  } finally {
    opts.free();
  }

  post({
    type: "converted",
    id,
    json: current.json,
    rawJson: current.rawJson ?? null,
    files: current.fileNames,
    imageCount: current.imageCount,
    elapsedMs: performance.now() - started,
  });
}

async function handleZip({ id }) {
  if (!current) throw new Error("Nothing has been converted yet.");
  post({ type: "status", id, stage: "Building ZIP…" });
  const zip = current.toZip();
  post({ type: "zip", id, zip }, [zip.buffer]);
}

self.onmessage = async ({ data }) => {
  try {
    if (data.type === "convert") await handleConvert(data);
    else if (data.type === "zip") await handleZip(data);
    else if (data.type === "version") {
      await ready;
      post({ type: "version", id: data.id, version: version() });
    }
  } catch (error) {
    post({ type: "error", id: data.id, ...describe(error) });
  }
};
