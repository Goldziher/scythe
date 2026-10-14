// AI-RULEZ :: GENERATED FILE — DO NOT EDIT
// Content-Hash: blake3:4c9a003b7651d77a972bef76d810e2fa5d4c4014b507da8d0053ec0e92b6f688
// Source-Hash: blake3:65e88be8faf16a3d5247c5be8b02ee1ae70158104d3025dd78a6b893899931bc
// Schema-Version: v1

/**
 * OpenCode v2 adapter for scythe.
 *
 * To add OpenCode-specific tools or hooks:
 *
 * 1. Create .ai-rulez/opencode/index.js.
 * 2. Default-export { id, setup(ctx) } from that source file. OpenCode v2 does not
 *    run v1 plugins (an exported function returning hooks).
 * 3. If the plugin bundles skills, commands, or agents, call
 *    registerBundledContent(ctx) from ../ai-rulez-content.js in setup.
 * 4. Run ai-rulez generate --plugin --dry-run, then ai-rulez generate --plugin.
 *
 * Keep shared skills, commands, agents, and MCP configuration in their normal
 * .ai-rulez sources. Validate all external input and never interpolate untrusted
 * values into shell commands.
 */
import { registerBundledContent } from "../ai-rulez-content.js"

/** @type {import("@opencode/plugin").Plugin.Plugin} */
export default {
  id: "scythe",
  async setup(ctx) {
    await registerBundledContent(ctx)
  },
}
