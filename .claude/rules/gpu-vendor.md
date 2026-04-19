# Rule: NTK GPU Vendor Selection

Applies to: any change touching `src/gpu.rs`, `src/config.rs`, `src/main.rs` setup wizard, or `src/compressor/layer3_llamacpp.rs`.

## Architecture

User choice in `ntk model setup` is persisted as:
- `config.model.gpu_vendor: Option<GpuVendor>` — `"nvidia"` | `"amd"` | `"apple"` | `null`
- `config.model.cuda_device: u32` — zero-based index **within the chosen vendor's namespace**

`device_id` is per-vendor. "NVIDIA device 0" and "AMD device 0" are different physical cards. The disambiguation comes from `gpu_vendor`.

## Runtime resolution

Always use `gpu::resolve_configured_backend(gpu_layers, gpu_vendor, cuda_device)` instead of `gpu::detect_best_backend()` when displaying status or choosing inference backend.

`detect_best_backend()` → auto-detects, **silently prefers NVIDIA** over AMD.
`resolve_configured_backend()` → honours the user's explicit choice, falls back to CPU (never cross-vendor).

## llama.cpp subprocess env vars

When spawning llama-server with a GPU selection, the env scoping is:
```
NVIDIA → CUDA_VISIBLE_DEVICES=<device_id>
AMD    → HIP_VISIBLE_DEVICES=<device_id>
         ROCR_VISIBLE_DEVICES=<device_id>
         GGML_VK_VISIBLE_DEVICES=<device_id>
```

This is done in `LlamaCppBackend::start()` via `.with_gpu_selection(vendor, device_id)`.

## Binary selection at install time (`ntk model install-server`)

`select_llama_cpp_asset(assets, vendor)` in `src/main.rs` picks the right
zip from the llama.cpp GitHub release based on `config.model.gpu_vendor`:

| Vendor | Preference order (filename tokens) |
|---|---|
| `Nvidia` | `cuda-13.1` > `cuda-12.4` > `cuda` > `vulkan` > `avx2` |
| `Amd` | `vulkan` > `hip-radeon` > `hip` > `avx2` |
| `Intel` | `sycl` > `vulkan` > `avx2` |
| `Apple` | first `macos` match (Metal is bundled) |
| `None` | `vulkan` > `avx2` |

Source of truth: `https://api.github.com/repos/ggml-org/llama.cpp/releases/latest`.
The repo was transferred from `ggerganov` to `ggml-org` in early 2026 —
do not revert the URL.

## setup_gpu_selection() return signature

Returns `(gpu_layers: i32, gpu_auto_detect: bool, device_id: u32, gpu_vendor: Option<GpuVendor>)`.

Both call sites (setup_candle + setup_llamacpp) must write all four fields to config, including `config.model.gpu_vendor = gpu_vendor`.
