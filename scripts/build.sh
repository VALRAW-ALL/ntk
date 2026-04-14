#!/usr/bin/env bash
# NTK auto-build — detects GPU/OS and invokes `cargo build` with the right
# Cargo feature flag.
#
#   NVIDIA + CUDA Toolkit installed   →  --features cuda
#   Apple Silicon (macOS aarch64)     →  --features metal
#   Everything else (AMD, Intel, ...) →  default features (Candle CPU)
#
# Pass through any extra cargo args, e.g.  ./build.sh --profile profiling

set -euo pipefail

feature=""

# --- Apple Silicon: Metal ----------------------------------------------------
if [[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "arm64" ]]; then
    feature="metal"
    echo "Detected: Apple Silicon → building with --features metal"
fi

# --- NVIDIA + CUDA Toolkit ---------------------------------------------------
if [[ -z "$feature" ]] && command -v nvidia-smi >/dev/null 2>&1; then
    if nvidia-smi --query-gpu=name --format=csv,noheader >/dev/null 2>&1; then
        if command -v nvcc >/dev/null 2>&1; then
            feature="cuda"
            echo "Detected: NVIDIA GPU + CUDA Toolkit → building with --features cuda"
        else
            echo "NVIDIA GPU detected but nvcc not in PATH — skipping --features cuda"
            echo "  Install the CUDA Toolkit and re-run to enable in-process CUDA."
        fi
    fi
fi

# --- Default path ------------------------------------------------------------
if [[ -z "$feature" ]]; then
    echo "No compatible in-process GPU backend detected — using default build (CPU + Ollama/llama.cpp external)."
    echo "  AMD users: inference runs through llama-server (Vulkan) or Ollama."
fi

cargo_args=(build --release)
[[ -n "$feature" ]] && cargo_args+=(--features "$feature")
cargo_args+=("$@")

echo "→ cargo ${cargo_args[*]}"
exec cargo "${cargo_args[@]}"
