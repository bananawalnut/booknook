// SPDX-License-Identifier: MIT OR Apache-2.0
// Test outputs use the same workspace-rooted Cargo metadata policy as packaging.
const path = require("node:path");
const {execFileSync} = require("node:child_process");
const root = path.resolve(__dirname, "..");
let directory;
function targetPath(...parts) {
  if (directory === undefined) {
    const output = execFileSync("python3", [path.join(root, "scripts/cargo_paths.py")], {
      cwd: root, encoding: "utf8", maxBuffer: 1024 * 1024, timeout: 10000,
    });
    // Python print adds one newline. Spaces at either end belong to the path.
    if (!output.endsWith("\n")) throw new Error("Cargo path helper must end with one newline");
    const value = output.slice(0, -1);
    if (!path.isAbsolute(value) || /[\0\r\n]/.test(value)) {
      throw new Error("Cargo path helper must return an absolute single-line path");
    }
    directory = value;
  }
  return path.join(directory, ...parts);
}
module.exports = {targetPath};
