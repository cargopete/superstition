#!/usr/bin/env bash
# End-to-end example run against the synthetic example corpus.
#
# Usage: bash examples/run.sh [--iterations N] [--hypotheses N]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CORPUS_DIR="$SCRIPT_DIR/corpus"
FEED_PATH="$SCRIPT_DIR/feed.json"
STATE_PATH="$SCRIPT_DIR/state.json"

ITERATIONS=3
HYPOTHESES=5

while [[ $# -gt 0 ]]; do
    case "$1" in
        --iterations|-n) ITERATIONS="$2"; shift 2 ;;
        --hypotheses|-h) HYPOTHESES="$2"; shift 2 ;;
        *) echo "unknown arg: $1"; exit 1 ;;
    esac
done

cd "$PROJECT_ROOT"

# ── 1. generate corpus if not already present ──────────────────────────────────
if [[ ! -f "$CORPUS_DIR/erc20_transfers.parquet" ]]; then
    echo "==> generating example corpus..."
    cargo run -q -p superstition-corpus --bin gen-example -- "$CORPUS_DIR"
    echo
fi

# ── 2. run the agent ───────────────────────────────────────────────────────────
echo "==> running agent  (iterations=$ITERATIONS  hypotheses=$HYPOTHESES)"
echo
cargo run --release -q -p superstition-agent -- \
    --corpus    "$CORPUS_DIR"  \
    --feed      "$FEED_PATH"   \
    --state     "$STATE_PATH"  \
    --workspace "$PROJECT_ROOT" \
    --iterations "$ITERATIONS" \
    --hypotheses "$HYPOTHESES"
