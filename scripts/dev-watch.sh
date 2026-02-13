#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────
# crumbeez development watcher
#
# Watches crates/ and Cargo.toml for changes and auto-rebuilds the
# WASM plugin.  After a successful build the new binary is at
#   target/wasm32-wasip1/debug/crumbeez.wasm
#
# To reload the running plugin press Ctrl+Shift+r (handled by the
# develop-rust-plugin helper loaded in the layout).
# ──────────────────────────────────────────────────────────────────
set -euo pipefail

BOLD='\033[1m'
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RESET='\033[0m'

banner() {
    echo -e "${BOLD}${CYAN}🐝 crumbeez dev-watch${RESET}"
    echo -e "${CYAN}────────────────────────────────────────${RESET}"
    echo -e "  Watching ${BOLD}crates/${RESET} and ${BOLD}Cargo.toml${RESET} for changes"
    echo ""
    echo -e "  ${BOLD}${YELLOW}┌─────────────────────────────────────┐${RESET}"
    echo -e "  ${BOLD}${YELLOW}│  Ctrl+Shift+r  →  rebuild & reload  │${RESET}"
    echo -e "  ${BOLD}${YELLOW}└─────────────────────────────────────┘${RESET}"
    echo ""
    echo -e "  ${CYAN}First launch?${RESET} Grant permissions to the"
    echo -e "  ${BOLD}develop-rust-plugin${RESET} pane below (press ${BOLD}y${RESET})."
    echo ""
}

# ── Ensure the WASM target is available ──────────────────────────
if ! rustup target list --installed 2>/dev/null | grep -q wasm32-wasip1; then
    echo -e "${YELLOW}⚠  wasm32-wasip1 target not installed. Adding...${RESET}"
    rustup target add wasm32-wasip1
fi

# ── Do an initial build so the plugin pane has something to load ─
initial_build() {
    if [ ! -f target/wasm32-wasip1/debug/crumbeez.wasm ]; then
        echo -e "${YELLOW}No WASM binary found — running initial build...${RESET}"
        cargo build 2>&1
        echo ""
    fi
}

# ── Prefer cargo-watch, fall back to a simple inotifywait loop ───
watch_with_cargo_watch() {
    exec cargo watch \
        --clear \
        --watch crates/ \
        --watch Cargo.toml \
        -s 'if cargo build 2>&1; then
                printf "\n\033[0;32m✅ Build succeeded\033[0m\n"
                printf "\033[1;33m   ╭──────────────────────────────────────╮\033[0m\n"
                printf "\033[1;33m   │  Press Ctrl+Shift+r to reload now!  │\033[0m\n"
                printf "\033[1;33m   ╰──────────────────────────────────────╯\033[0m\n"
            else
                printf "\n\033[0;31m❌ Build failed\033[0m  —  fix the errors above\n"
            fi'
}

watch_with_inotifywait() {
    echo -e "${YELLOW}(using inotifywait fallback — install cargo-watch for a better experience)${RESET}"
    echo ""
    while true; do
        echo -e "${CYAN}Waiting for file changes...${RESET}"
        inotifywait -r -q -e modify,create,delete,move crates/ Cargo.toml 2>/dev/null
        echo -e "\n${YELLOW}Change detected — building...${RESET}\n"
        if cargo build 2>&1; then
            echo -e "\n${GREEN}✅ Build succeeded${RESET}"
            echo -e "${BOLD}${YELLOW}   ╭──────────────────────────────────────╮${RESET}"
            echo -e "${BOLD}${YELLOW}   │  Press Ctrl+Shift+r to reload now!  │${RESET}"
            echo -e "${BOLD}${YELLOW}   ╰──────────────────────────────────────╯${RESET}"
        else
            echo -e "\n${RED}❌ Build failed${RESET}  —  fix the errors above"
        fi
        echo ""
    done
}

# ── Main ─────────────────────────────────────────────────────────
banner
initial_build

if command -v cargo-watch &>/dev/null; then
    watch_with_cargo_watch
elif command -v inotifywait &>/dev/null; then
    watch_with_inotifywait
else
    echo -e "${RED}Error: neither cargo-watch nor inotifywait found.${RESET}"
    echo -e "Install one of them:"
    echo -e "  ${BOLD}cargo install cargo-watch${RESET}   (recommended)"
    echo -e "  ${BOLD}sudo apt install inotify-tools${RESET}"
    exit 1
fi

