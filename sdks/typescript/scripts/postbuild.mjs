// Mark the emitted dirs with their module systems so Node resolves each
// correctly regardless of the root package.json "type".
import { writeFileSync } from "node:fs";

writeFileSync("dist/cjs/package.json", JSON.stringify({ type: "commonjs" }) + "\n");
writeFileSync("dist/esm/package.json", JSON.stringify({ type: "module" }) + "\n");
