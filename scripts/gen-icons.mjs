// Generates the Lucide data used by the UI (rendering icons named in app
// manifests) and the backend (validating those names).
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const nodes = JSON.parse(readFileSync(require.resolve("lucide-static/icon-nodes.json"), "utf8"));

writeFileSync(new URL("../src/lib/lucide-nodes.json", import.meta.url), JSON.stringify(nodes));
writeFileSync(new URL("../src-tauri/src/lucide-names.txt", import.meta.url), Object.keys(nodes).sort().join("\n") + "\n");
console.log(`lucide: ${Object.keys(nodes).length} icons`);
