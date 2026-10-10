#!/usr/bin/env bash
# SessionStart: make a fresh Claude Code on the web container ready for the
# gate with no manual setup. Local sessions already have their toolchain, so
# the hook does nothing outside a remote session. It is safe on a repo with no
# Cargo.toml yet, never installs heavy optional tools (it prints how instead),
# and never runs `playwright install`: the image ships Chromium under
# /opt/pw-browsers.
set -uo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(dirname "$0")/../..}" || exit 0

env_line() {
  if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
    echo "$1" >>"$CLAUDE_ENV_FILE"
  fi
}

# The pinned toolchain. rustup >= 1.28 installs the toolchain named by
# rust-toolchain.toml (with its components) when given no argument.
if [ -f rust-toolchain.toml ] || [ -f rust-toolchain ]; then
  if command -v rustup >/dev/null 2>&1; then
    if ! log="$(rustup toolchain install 2>&1)"; then
      printf 'rustup could not install the pinned toolchain:\n%s\n' "$log" >&2
      exit 1
    fi
    echo "rust: $(rustc --version 2>/dev/null) (pinned by rust-toolchain.toml)"
  else
    echo "rust: rustup missing; install it from https://rustup.rs, then run \`rustup toolchain install\`"
  fi
else
  echo "rust: $(rustc --version 2>/dev/null || echo missing) (no rust-toolchain.toml yet)"
fi

# Crates, so the first build in a worktree does not wait on the network.
if [ -f Cargo.lock ] && [ -f Cargo.toml ]; then
  if ! log="$(cargo fetch --locked 2>&1)"; then
    printf 'cargo fetch --locked failed:\n%s\n' "$(printf '%s\n' "$log" | tail -n 30)" >&2
    exit 1
  fi
  echo "crates: fetched (Cargo.lock)"
elif [ -f Cargo.toml ]; then
  echo "crates: no Cargo.lock yet; the first build will create it"
else
  echo "crates: no Cargo.toml yet; nothing to fetch"
fi

# Browser checks use the preinstalled Chromium.
if [ -d /opt/pw-browsers ] && [ -z "${PLAYWRIGHT_BROWSERS_PATH:-}" ]; then
  env_line "export PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers"
fi

# Optional tools: report, never fail, never install the heavy ones here.
for tool in cargo-nextest cargo-deny cargo-llvm-cov cargo-mutants; do
  if command -v "$tool" >/dev/null 2>&1; then
    echo "$tool: present"
  else
    echo "$tool: missing; install with \`cargo install --locked ${tool}\` when a gate needs it"
  fi
done
if [ -x formal/node_modules/.bin/quint ] || command -v quint >/dev/null 2>&1; then
  echo "quint: present"
elif [ -f formal/package-lock.json ]; then
  echo "quint: missing; run \`npm ci --prefix formal\` (dev-time only; Node $(node --version 2>/dev/null || echo missing))"
else
  echo "quint: not needed yet (no formal/package-lock.json)"
fi

if [ -f Cargo.toml ]; then
  if grep -qsE '^[[:space:]]*xtask[[:space:]]*=' .cargo/config.toml; then
    echo "site-preview ready: run \`cargo xtask quick\` while working, \`cargo xtask check\` before commit (see doc/tasks.md)"
  else
    echo "site-preview ready: gate is the fallback in CLAUDE.md until xtask exists"
  fi
else
  echo "site-preview: no code yet; see doc/tasks.md for the first task"
fi
exit 0
