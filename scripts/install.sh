#!/bin/sh
# NTK system installer — macOS / Linux
#
# Usage:
#   curl -sSf https://ntk.valraw.com/install.sh | sh
#
# Optional non-interactive override (pipeline installs):
#   NTK_INSTALL_PLATFORM=nvidia|amd|cpu  — skip the interactive prompt.
#
# The script:
#   1. Enumerates discrete GPUs on the system (NVIDIA and AMD, any number).
#   2. If STDIN is a TTY, shows a numbered list and lets you pick a release
#      variant (NVIDIA / AMD / CPU-only). Non-interactive defaults to NVIDIA
#      when an NVIDIA GPU is detected, otherwise CPU-only.
#   3. Downloads the matching artifact (`ntk-<os>-<arch>-{cpu,gpu}`) from the
#      latest GitHub release and installs it to /usr/local/bin/ntk.

set -e

REPO="VALRAW-ALL/ntk"

# ---------------------------------------------------------------------------
# Host info
# ---------------------------------------------------------------------------

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
case "$ARCH" in
    x86_64)         ARCH="x86_64"  ;;
    arm64|aarch64)  ARCH="aarch64" ;;
    *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

# ---------------------------------------------------------------------------
# GPU enumeration
# ---------------------------------------------------------------------------

detect_nvidia_gpus() {
    # One line per GPU: "<name>|<vram_mb>". Silent when no GPU or no driver.
    if command -v nvidia-smi >/dev/null 2>&1; then
        nvidia-smi --query-gpu=name,memory.total \
                   --format=csv,noheader,nounits 2>/dev/null \
            | awk -F', *' 'NF>=2 {printf "%s|%s\n", $1, $2}'
        return
    fi
    # Fallback: driver absent but card may still be present on the PCI bus.
    if command -v lspci >/dev/null 2>&1; then
        lspci -nn 2>/dev/null | awk -F': ' '
            /VGA|3D|Display/ && /NVIDIA/ { sub(/ \[[0-9a-f:]+\]$/,"",$NF); print $NF "|0" }
        '
    fi
}

detect_amd_gpus() {
    # Linux: prefer sysfs (VRAM accurate for any card). macOS: skip — all AMD
    # Macs are dGPU/eGPU setups and Candle doesn't target them.
    if [ "$OS" = "linux" ]; then
        # sysfs walk
        for card in /sys/class/drm/card*/device; do
            [ -e "$card/vendor" ] || continue
            [ "$(cat "$card/vendor" 2>/dev/null)" = "0x1002" ] || continue
            did=$(cat "$card/device" 2>/dev/null | sed 's/^0x//')
            vram=0
            [ -r "$card/mem_info_vram_total" ] \
                && vram=$(awk '{printf "%d", $1/1048576}' "$card/mem_info_vram_total")
            name="AMD GPU"
            if command -v lspci >/dev/null 2>&1 && [ -n "$did" ]; then
                name=$(lspci -nn -d "1002:$did" 2>/dev/null \
                       | head -1 \
                       | sed -e 's/.*: //' -e 's/ \[[0-9a-f:]*\]$//')
                [ -z "$name" ] && name="AMD GPU"
            fi
            printf "%s|%s\n" "$name" "$vram"
        done
    fi
}

NVIDIA_GPUS=$(detect_nvidia_gpus || true)
AMD_GPUS=$(detect_amd_gpus || true)

HAS_NVIDIA=0; [ -n "$NVIDIA_GPUS" ] && HAS_NVIDIA=1
HAS_AMD=0;    [ -n "$AMD_GPUS" ]    && HAS_AMD=1

# ---------------------------------------------------------------------------
# Platform selection
# ---------------------------------------------------------------------------

echo ""
echo "  NTK installer"
echo "  ─────────────"
echo ""

print_gpu_list() {
    i=1
    if [ "$HAS_NVIDIA" = "1" ]; then
        echo "$NVIDIA_GPUS" | while IFS='|' read -r name vram; do
            [ -z "$name" ] && continue
            if [ "$vram" != "0" ] && [ -n "$vram" ]; then
                echo "    GPU #${i}  NVIDIA  ${name}  (${vram} MB VRAM)"
            else
                echo "    GPU #${i}  NVIDIA  ${name}"
            fi
            i=$((i+1))
        done
    fi
    if [ "$HAS_AMD" = "1" ]; then
        echo "$AMD_GPUS" | while IFS='|' read -r name vram; do
            [ -z "$name" ] && continue
            if [ "$vram" != "0" ] && [ -n "$vram" ]; then
                echo "    GPU #${i}  AMD     ${name}  (${vram} MB VRAM)"
            else
                echo "    GPU #${i}  AMD     ${name}"
            fi
            i=$((i+1))
        done
    fi
}

if [ "$HAS_NVIDIA" = "1" ] || [ "$HAS_AMD" = "1" ]; then
    echo "  Detected GPUs:"
    print_gpu_list
    echo ""
fi

# Default choice based on detection: NVIDIA > CPU (AMD uses CPU binary + external llama-server)
if [ "$HAS_NVIDIA" = "1" ]; then
    DEFAULT_PLATFORM="nvidia"
else
    DEFAULT_PLATFORM="cpu"
fi

PLATFORM="${NTK_INSTALL_PLATFORM:-}"

if [ -z "$PLATFORM" ]; then
    if [ -t 0 ] && [ -t 1 ]; then
        echo "  Which release do you want to install?"
        echo "    [1] NVIDIA (GPU build, CUDA)"
        echo "    [2] AMD    (CPU build + configure llama-server Vulkan later)"
        echo "    [3] CPU only"
        echo ""
        case "$DEFAULT_PLATFORM" in
            nvidia) def=1 ;;
            amd)    def=2 ;;
            *)      def=3 ;;
        esac
        printf "  Choose [1/2/3] or Enter for [%s]: " "$def"
        read -r choice
        [ -z "$choice" ] && choice="$def"
        case "$choice" in
            1) PLATFORM="nvidia" ;;
            2) PLATFORM="amd"    ;;
            3) PLATFORM="cpu"    ;;
            *) echo "Invalid choice." >&2; exit 1 ;;
        esac
    else
        # Non-interactive: use detection default.
        PLATFORM="$DEFAULT_PLATFORM"
        echo "  Non-interactive install — selecting '${PLATFORM}' automatically."
        echo "  (Set NTK_INSTALL_PLATFORM=nvidia|amd|cpu to override.)"
    fi
fi

# Map user choice → artifact suffix.
#   nvidia → GPU binary (CUDA / Metal on Apple)
#   amd    → CPU binary (the NTK binary has no AMD backend; inference uses
#            an external llama-server built with Vulkan, configured later)
#   cpu    → CPU binary
case "$PLATFORM" in
    nvidia)
        if [ "$OS" = "darwin" ] && [ "$ARCH" = "aarch64" ]; then
            # Apple Silicon has no CUDA; map to the Metal GPU build.
            SUFFIX="gpu"
            echo "  Note: on Apple Silicon the 'nvidia' choice maps to the Metal GPU build."
        else
            SUFFIX="gpu"
        fi
        ;;
    amd)
        SUFFIX="cpu"
        POST_INSTALL_NOTE="AMD"
        ;;
    cpu)
        SUFFIX="cpu"
        ;;
    *)
        echo "Invalid NTK_INSTALL_PLATFORM='$PLATFORM' (expected nvidia|amd|cpu)" >&2
        exit 1
        ;;
esac

# ---------------------------------------------------------------------------
# Download + install
# ---------------------------------------------------------------------------

LATEST=$(curl -sSf "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' | cut -d'"' -f4)

ARTIFACT="ntk-${OS}-${ARCH}-${SUFFIX}"
URL="https://github.com/${REPO}/releases/download/${LATEST}/${ARTIFACT}"
DEST="/usr/local/bin/ntk"

echo ""
echo "  Downloading ${ARTIFACT} (${LATEST})…"
if ! curl -sSfL "$URL" -o /tmp/ntk; then
    echo "Download failed." >&2
    echo "  URL: $URL" >&2
    echo "  This variant may not exist for your platform — try another choice." >&2
    exit 1
fi
chmod +x /tmp/ntk

if [ -w /usr/local/bin ]; then
    mv /tmp/ntk "$DEST"
else
    sudo mv /tmp/ntk "$DEST"
fi

echo ""
echo "  NTK installed to $DEST"
echo ""

if [ "${POST_INSTALL_NOTE:-}" = "AMD" ]; then
    echo "  ⚠  AMD GPU note: inference uses llama-server + Vulkan (external)."
    echo "     Install llama.cpp (Vulkan build) from:"
    echo "       https://github.com/ggerganov/llama.cpp/releases"
    echo "     Place 'llama-server' on your PATH (or ~/.ntk/bin/) before"
    echo "     running 'ntk model setup' — choose 'llama.cpp' backend."
    echo ""
fi

# ---------------------------------------------------------------------------
# Step 1 — ntk init -g  (register PostToolUse hook in Claude Code)
# ---------------------------------------------------------------------------
echo "  ── Step 1/2: Initializing NTK hook (ntk init -g) ──"
echo ""
ntk init -g
echo ""

# ---------------------------------------------------------------------------
# Step 2 — ntk model setup  (configure backend + GPU)
# ---------------------------------------------------------------------------
echo "  ── Step 2/2: Configuring inference backend (ntk model setup) ──"
echo ""
ntk model setup
echo ""
echo "  ✓ Installation complete. Run  ntk start  to launch the daemon."
echo ""
