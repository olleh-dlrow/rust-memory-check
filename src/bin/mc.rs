/*
 * @Author: Shuwen Chen 
 * @Date: 2023-03-12 16:45:12 
 * @Last Modified by: Shuwen Chen
 * @Last Modified time: 2023-03-13 02:03:09
 */

fn main() {
    rust_memory_check::core::utils::init_log(log::Level::Debug).expect("init log failed");
    std::process::exit(rust_memory_check::analysis_then_check().is_err() as i32);
}
