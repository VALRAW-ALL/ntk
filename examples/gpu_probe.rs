fn main() {
    let gpus = ntk::gpu::enumerate_gpus();
    println!("enumerate_gpus() returned {} device(s):", gpus.len());
    for g in &gpus {
        println!("  - {:?}", g);
    }
    println!("detect_best_backend() = {:?}", ntk::gpu::detect_best_backend());
    println!("gpu_model_name() = {:?}", ntk::gpu::gpu_model_name());
}
