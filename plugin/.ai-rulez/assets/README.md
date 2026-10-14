# Scythe coding-agent plugin

This plugin supplies one Scythe skill and five reference documents. Install the
Scythe CLI separately (`cargo install scythe-cli --version 0.20.0`). It registers
no MCP tools and installs no database drivers.

The native ai-rulez authoring source is `plugin/.ai-rulez/`. All eleven runtime
formats are generated from that source: Claude, Cursor, Codex, Gemini, Kimi,
OpenCode, Factory, Hermes, Pi, Agent Plugins and Copilot.

## Install

Claude: `/plugin marketplace add Goldziher/scythe`, then
`/plugin install scythe@scythe`.

Codex: `codex plugin marketplace add Goldziher/scythe`, then
`codex plugin add scythe@scythe`.

For other runtimes, clone this repository and install the local generated bundle:

| Runtime | Local bundle |
| --- | --- |
| Cursor | `plugin/` with `.cursor-plugin/plugin.json` and its bundled resources |
| Gemini | `plugin/` with `gemini-extension.json` and `assets/GEMINI.md` |
| Kimi | `plugin/` with `kimi.plugin.json` |
| OpenCode v2 | `plugin/` as a local package; entry `.opencode/plugins/scythe.js` |
| Factory | `plugin/` with `.factory-plugin/plugin.json` |
| Hermes | `plugin/.hermes/plugins/scythe/`, or install the local `plugin/.hermes/package/` wheel project |
| Pi | `plugin/` as a local Pi package (its package.json declares skills) |
| Agent Plugins / Copilot | `plugin/plugin.json`, discovered from `.agents/plugins/marketplace.json` |

No npm or Python plugin package has been published by this migration. Use local
paths until a separately reviewed registry release exists. Public Agent Skills
are exported under `skills/scythe/` with their references included.

## Regenerate and verify

Use ai-rulez 5.3.0 from reviewed source revision
`1dd46d774f156f3d170ecd1b702598b12d1eaf5b`.
Run `task plugin:generate` after source edits and `task plugin:verify` to verify
source provenance, runtime contracts, version alignment and public export bytes.

The manually dispatched plugin-artifacts workflow verifies the selected checkout
against an existing release tag and runs three distribution jobs: a deterministic
GitHub archive, the npm OpenCode/Pi package and the PyPI Hermes wheel. It does not
create a product tag. Configure the required registry trusted publishers before
dispatching; see [plugin publishing](https://github.com/Goldziher/scythe/blob/main/.github/PLUGIN-PUBLISHING.md).

Gemini context loading follows the [official extension manifest contract](https://geminicli.com/docs/extensions/reference/).
