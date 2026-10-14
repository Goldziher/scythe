import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import plugin from "../../plugin/.opencode/plugins/scythe.js";

const root = path.resolve(import.meta.dirname, "../..");

test("OpenCode registers the installed Scythe skill once with readable references", async () => {
  const registered = new Map();
  const context = {
    skill: {
      transform(callback) {
        callback({ get: (id) => registered.get(id), add: (skill) => registered.set(skill.id, skill) });
      },
    },
  };
  await plugin.setup(context);
  await plugin.setup(context);
  assert.equal(registered.size, 1);
  const skill = registered.get("scythe/scythe");
  assert.equal(skill.name, "scythe");
  assert.match(skill.content, /scythe generate/u);
  for (const resource of ["annotations", "backends", "cli-reference", "configuration", "lint-rules"]) {
    const content = await readFile(path.join(path.dirname(skill.path), `references/${resource}.md`), "utf8");
    assert.ok(content.length > 100, `resource missing content: ${resource}`);
  }
});

test("Pi package ships every declared skill and its five references", async () => {
  const manifest = JSON.parse(await readFile(path.join(root, "plugin/package.json"), "utf8"));
  assert.deepEqual(manifest.pi.skills, ["./.pi/skills/scythe"]);
  assert.deepEqual(manifest.pi.prompts, []);
  for (const item of [
    "SKILL.md",
    "index.md",
    ...["annotations", "backends", "cli-reference", "configuration", "lint-rules"].map(
      (name) => `references/${name}.md`,
    ),
  ]) {
    const file = `.pi/skills/scythe/${item}`;
    assert.ok(manifest.files.includes(file), `npm file list omitted ${file}`);
    assert.ok((await readFile(path.join(root, "plugin", file))).length > 0);
  }
});
