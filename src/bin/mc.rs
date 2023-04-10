fn main() {
    rust_memory_check::core::utils::init_log(log::Level::Debug).expect("init log failed");
    std::process::exit(rust_memory_check::analysis_then_check().is_err() as i32);
}
