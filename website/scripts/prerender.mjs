import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import vm from "node:vm";
import { render } from "../dist-ssr/prerender.js";
import { PRERENDER_TARGETS, parseTargetString, renderBounceHtml } from "../src/deepLinks.ts";

const root = resolve(import.meta.dirname, "..");
const indexPath = join(root, "dist", "index.html");
let html = readFileSync(indexPath, "utf8");
const marker = '<div id="root"></div>';

if (!html.includes(marker)) {
  throw new Error(`dist/index.html is missing ${marker}`);
}

// the one small stylesheet goes inline, so the first paint needs only the page
const sheet = /<link rel="stylesheet"[^>]*href="[^"]*assets\/([^"]+\.css)"[^>]*>/;
const css = sheet.exec(html);
if (!css) throw new Error("dist/index.html has no stylesheet to inline");
const style = readFileSync(join(root, "dist", "assets", css[1]), "utf8").replaceAll("</style", "<\\/style");
html = html.replace(css[0], () => `<style>${style}</style>`);

writeFileSync(indexPath, html.replace(marker, () => `<div id="root">${render()}</div>`));

function assertValidInlineScript(html, label) {
  const match = /<script>([\s\S]*?)<\/script>/.exec(html);
  if (!match) throw new Error(`missing inline script in bounce page: ${label}`);
  new vm.Script(match[1], { filename: `bounce:${label}` });
}

// Emit static dark-mode deep-link bounce pages (/open/, /labs/<slug>, /features/<slug>, /settings/<pane>).
const defaultTarget = parseTargetString("settings");
if (!defaultTarget) throw new Error("failed to build default settings bounce target");
const openDir = join(root, "dist", "open");
mkdirSync(openDir, { recursive: true });
const openHtml = renderBounceHtml(defaultTarget, { allowQueryOverride: true });
assertValidInlineScript(openHtml, "/open/");
writeFileSync(join(openDir, "index.html"), openHtml);
for (const route of PRERENDER_TARGETS) {
  const target = parseTargetString(route.slice(1));
  if (!target) throw new Error(`invalid prerender deep-link route: ${route}`);
  const dir = join(root, "dist", ...route.slice(1).split("/"));
  mkdirSync(dir, { recursive: true });
  const pageHtml = renderBounceHtml(target);
  assertValidInlineScript(pageHtml, route);
  writeFileSync(join(dir, "index.html"), pageHtml);
}

rmSync(join(root, "dist-ssr"), { recursive: true, force: true });
