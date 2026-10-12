#!/usr/bin/env bash
# Capture desktop + mobile screenshots of the running app.
#
# Inputs (environment):
#   CAPTURE_URL  exact URL to open in the browser (required).
#   CAPTURE_DIR  directory receiving final-desktop.png / final-mobile.png (required).
#
# Behavior:
# - Opens the URL in its own headed Chromium (SwiftShader software GL),
#   waits for the 3D canvas to actually render, screenshots both viewports,
#   then closes its browser. The app server is left running.
# - Exit 75: temporary navigation / browser infrastructure failure.
# - Exit 1: script defect or rendering defect (page loads but shows no scene).
set -euo pipefail
cd "$(dirname "$0")"

: "${CAPTURE_URL:?Set CAPTURE_URL to the exact URL to capture.}"
: "${CAPTURE_DIR:?Set CAPTURE_DIR to the screenshot output directory.}"

/usr/bin/time -p mkdir -p "$CAPTURE_DIR"

/usr/bin/time -p node --input-type=module - <<'NODE_EOF'
import { readFileSync, mkdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { createRequire } from 'node:module';

const runtime = join(process.env.HOME, '.local/share/omgithub-playwright');
const require = createRequire(join(runtime, 'package.json'));
const { chromium } = require('playwright');
const config = JSON.parse(readFileSync(join(runtime, 'linux.json'), 'utf8'));
if (!process.env.DISPLAY) {
  process.env.DISPLAY = ':' + readFileSync(join(runtime, 'display'), 'utf8').trim();
}

const url = process.env.CAPTURE_URL;
const output = process.env.CAPTURE_DIR;
if (!url || !output) {
  console.error('Set CAPTURE_URL and CAPTURE_DIR.');
  process.exit(1);
}
mkdirSync(output, { recursive: true });

// 75 = temporary navigation / browser infrastructure failure.
const transient = (error) => {
  throw Object.assign(error instanceof Error ? error : new Error(String(error)), { exitCode: 75 });
};

let browser;
const pageErrors = [];
try {
  browser = await chromium.launch({ ...config.browser.launchOptions, timeout: 30000 }).catch(transient);
  for (const [name, width, height] of [['desktop', 1440, 900], ['mobile', 390, 844]]) {
    const page = await browser.newPage({ viewport: { width, height } }).catch(transient);
    try {
      page.setDefaultTimeout(30000);
      page.on('pageerror', (error) => pageErrors.push(`${name}: ${error?.message || error}`));
      const response = await page.goto(url, { waitUntil: 'load', timeout: 60000 }).catch(transient);
      if (!response?.ok()) {
        throw Object.assign(new Error(`HTTP ${response?.status()} loading preview`), {
          exitCode: !response || [408, 429, 500, 502, 503, 504].includes(response.status()) ? 75 : 1,
        });
      }
      // Wait for the game canvas. The page loaded, so a missing canvas is a
      // rendering defect (exit 1), not infrastructure.
      try {
        await page.locator('#bevy').waitFor({ state: 'visible', timeout: 90000 });
      } catch (error) {
        throw Object.assign(new Error(`game canvas #bevy never appeared: ${error?.message || error}`), { exitCode: 1 });
      }
      // Prove the WebGL frame loop is running by counting rAF callbacks.
      try {
        const framesMs = await page.evaluate(() => new Promise((resolve, reject) => {
          const t0 = performance.now();
          let n = 0;
          function f() {
            n += 1;
            if (n >= 45) resolve(Math.round(performance.now() - t0));
            else if (performance.now() - t0 > 120000) reject(new Error('render loop stall: 45 frames not reached in 120s'));
            else requestAnimationFrame(f);
          }
          requestAnimationFrame(f);
        }));
        console.log(`Capture ${name}: 45 frames in ${framesMs}ms`);
      } catch (error) {
        throw Object.assign(new Error(`no running frame loop: ${error?.message || error}`), { exitCode: 1 });
      }
      // Soak so software GL paints the full scene before the shot.
      await page.waitForTimeout(5000);
      const shot = join(output, `final-${name}.png`);
      await page.screenshot({ path: shot, timeout: 30000 }).catch((error) => {
        if (error?.name === 'TimeoutError' || !browser.isConnected()) transient(error);
        throw error;
      });
      const size = statSync(shot).size;
      console.log(`Capture ${name}: wrote ${shot} (${size} bytes)`);
      if (size < 20000) {
        throw Object.assign(new Error(`rendering defect: ${shot} is only ${size} bytes`), { exitCode: 1 });
      }
    } finally {
      await page.close().catch(() => {});
    }
  }
} catch (error) {
  console.error(error?.message || error);
  process.exitCode = error?.exitCode || 1;
} finally {
  await browser?.close().catch((error) => {
    console.error(error?.message || error);
    process.exitCode ||= 75;
  });
}
if (pageErrors.length > 0) {
  console.error(`page errors observed:\n${pageErrors.join('\n')}`);
}
NODE_EOF
code=$?
echo "capture.sh exit: $code (url=$CAPTURE_URL dir=$CAPTURE_DIR)"
exit "$code"
