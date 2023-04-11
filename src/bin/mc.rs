fn main() {
    std::process::exit(rust_memory_check::analysis_then_check().is_err() as i32);
}
