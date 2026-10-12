#!/usr/bin/env bash
# Build (when needed) and serve the browser (WASM) build of Playground 3D.
#
# Contract:
# - Runs in the foreground serving static files from PROJECT_DIR/dist.
# - Writes ${OPENCODE_WEB_DIR}/deployment-output.json =
#   {"project": "<abs project dir>", "directory": "<abs dist dir>"}.
# - Source and built output stay inside PROJECT_DIR; OPENCODE_WEB_DIR and
#   RUNNER_TEMP are used only for worker metadata / capture evidence.
set -euo pipefail
cd "$(dirname "$0")"

PROJECT_DIR="$PWD"
DIST_DIR="$PROJECT_DIR/dist"
WEB_DIR="${OPENCODE_WEB_DIR:-/home/runner/work/_temp/omgithub-web}"
PORT="${PORT:-3000}"
TRUNK_VERSION="v0.21.14"
export PROJECT_DIR DIST_DIR WEB_DIR PORT

/usr/bin/time -p bash -c 'test -n "'"${PROJECT_DIR}"'" && test -d "'"${PROJECT_DIR}"'/src"'

# 1) Install dependencies (idempotent; mirrors startup.sh so start.sh is
#    self-sufficient even if the once-per-worker hook has not run yet).
/usr/bin/time -p bash -c '
  set -euo pipefail
  command -v node >/dev/null 2>&1 || { echo "node is required to serve dist/" >&2; exit 1; }
  command -v cargo >/dev/null 2>&1 || { echo "cargo is required to build the game" >&2; exit 1; }
  if ! rustup target list --installed 2>/dev/null | grep -q "^wasm32-unknown-unknown$"; then
    rustup target add wasm32-unknown-unknown
  fi
  if ! command -v trunk >/dev/null 2>&1; then
    tmp="$(mktemp -d)"
    trap "rm -rf $tmp" EXIT
    curl --fail --silent --show-error --location --max-time 120 \
      "https://github.com/trunk-rs/trunk/releases/download/'"$TRUNK_VERSION"'/trunk-x86_64-unknown-linux-gnu.tar.gz" \
      -o "$tmp/trunk.tar.gz"
    tar -xzf "$tmp/trunk.tar.gz" -C "$tmp"
    mkdir -p "$HOME/.cargo/bin"
    install -m 0755 "$tmp/trunk" "$HOME/.cargo/bin/trunk"
  fi
  export PATH="$HOME/.cargo/bin:$PATH"
  trunk --version
'

# 2) Build when needed: missing dist/index.html, or any input newer than it.
needs_build=0
if [[ ! -f "$DIST_DIR/index.html" ]]; then
  needs_build=1
else
  if [[ -n "$(find src index.html Trunk.toml Cargo.toml Cargo.lock -newer "$DIST_DIR/index.html" -print -quit 2>/dev/null)" ]]; then
    needs_build=1
  fi
fi
if [[ "$needs_build" == 1 ]]; then
  echo "Building browser bundle with trunk..."
  /usr/bin/time -p env PATH="$HOME/.cargo/bin:$PATH" trunk build --release
else
  echo "dist/ is up to date; skipping trunk build."
fi
/usr/bin/time -p test -f "$DIST_DIR/index.html"

# 3) Publish deployment metadata for the controller (worker metadata only).
/usr/bin/time -p mkdir -p "$WEB_DIR"
/usr/bin/time -p node -e 'const fs = require("node:fs"); fs.mkdirSync(process.env.WEB_DIR, { recursive: true }); fs.writeFileSync(process.env.WEB_DIR + "/deployment-output.json", JSON.stringify({ project: process.env.PROJECT_DIR, directory: process.env.DIST_DIR })); console.log(fs.readFileSync(process.env.WEB_DIR + "/deployment-output.json", "utf8"));'

# 4) Serve dist/ in the foreground on PORT.
echo "Serving $DIST_DIR on 0.0.0.0:$PORT ..."
exec node --input-type=module - <<'NODE_EOF'
import { createServer } from 'node:http';
import { readFileSync, statSync } from 'node:fs';
import { resolve, join, extname } from 'node:path';

const root = resolve(process.env.DIST_DIR);
const port = Number(process.env.PORT || 3000);
const mime = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.mjs': 'application/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.wasm': 'application/wasm',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.webp': 'image/webp',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
};

const server = createServer((req, res) => {
  try {
    const url = new URL(req.url, 'http://localhost');
    let path = resolve(root, '.' + decodeURIComponent(url.pathname));
    if (path !== root && !path.startsWith(root + '/')) {
      res.writeHead(404);
      res.end('Not found');
      return;
    }
    try {
      if (statSync(path).isDirectory()) path = join(path, 'index.html');
    } catch {
      res.writeHead(404);
      res.end('Not found');
      return;
    }
    res.setHeader('Content-Type', mime[extname(path)] || 'application/octet-stream');
    res.setHeader('Cache-Control', 'no-cache');
    res.end(readFileSync(path));
  } catch {
    res.writeHead(404);
    res.end('Not found');
  }
});

server.listen(port, '0.0.0.0', () => console.log(`playground-3d serving ${root} on 0.0.0.0:${port}`));
NODE_EOF
