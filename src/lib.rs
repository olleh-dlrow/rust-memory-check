/*
 * @Author: Shuwen Chen 
 * @Date: 2023-03-13 00:14:58 
 * @Last Modified by: Shuwen Chen
 * @Last Modified time: 2023-03-13 02:21:13
 */

#![feature(rustc_private)]
#![feature(box_patterns)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;
extern crate rustc_errors;

pub mod core {
    pub mod utils;
    pub mod check;
    pub mod cfg;
    pub mod analysis;
}

use crate::core::cfg::ControlFlowGraph;

pub fn analysis_then_check() -> Result<(), rustc_errors::ErrorGuaranteed> {
    rustc_driver::catch_fatal_errors(move || {
        let rustc_args = get_rustc_args();
        let mut callbacks = MemoryCheckCallbacks;
        rustc_driver::RunCompiler::new(&rustc_args, &mut callbacks).run()
    }).and_then(|result| result)
}

struct MemoryCheckCallbacks;

impl rustc_driver::Callbacks for MemoryCheckCallbacks {
    fn after_analysis<'tcx>(
        &mut self,
        compiler: &rustc_interface::interface::Compiler,
        queries: &'tcx rustc_interface::Queries<'tcx>,
    ) -> rustc_driver::Compilation {
        compiler.session().abort_if_errors();

        queries.global_ctxt().unwrap().peek_mut().enter(|tcx| {
            tcx.hir().par_body_owners(|local_def_id| analysis_then_check_body(tcx, local_def_id.to_def_id()));
        });
        rustc_driver::Compilation::Continue
    }
}

fn analysis_then_check_body(_tcx: rustc_middle::ty::TyCtxt, _def_id: rustc_hir::def_id::DefId) {
    let mut cfg = ControlFlowGraph::new();
    crate::core::analysis::alias_analysis(&mut cfg);
    let _report = crate::core::check::check_then_report(&mut cfg);
}

fn get_rustc_args() -> Vec<String> {
    let rustc_args = std::env::args().into_iter().collect::<Vec<String>>();

    match get_compile_time_sysroot() {
        Some(sysroot) => {
            let sysroot_flag = "--sysroot";
            if !rustc_args.iter().any(|arg| arg == sysroot_flag) {
                rustc_args.into_iter().chain(vec![sysroot_flag.to_owned(), sysroot]).collect()
            } else {
                rustc_args
            }
        },
        None => rustc_args
    }
}

fn get_compile_time_sysroot() -> Option<String> {
    if option_env!("RUST_STAGE").is_some() {
        return None;
    }
    let home = option_env!("RUSTUP_HOME").or(option_env!("MULTIRUST_HOME"));
    let toolchain = option_env!("RUSTUP_TOOLCHAIN").or(option_env!("MULTIRUST_TOOLCHAIN"));
    Some(match (home, toolchain) {
        (Some(home), Some(toolchain)) => format!("{}/toolchains/{}", home, toolchain),
        _ => option_env!("RUST_SYSROOT")
            .expect(
                "To build without rustup, set the `RUST_SYSROOT` env var at build time",
            )
            .to_owned(),
    })
}


#[cfg(test)]
mod tests {
    use crate::core::utils;
    #[test]
    fn test_log() {
        const DEBUG_INFO:& str = "TEST LOG";

        assert!(utils::init_log(log::Level::Debug).is_ok());
        log::debug!("{}", DEBUG_INFO);      
        
        let tmp_dir = std::env::temp_dir();
        let log_path = tmp_dir.as_path().join(utils::LOG_FILENAME);
        assert!(tmp_dir.as_path().join(utils::CONFIG_FILENAME).exists());
        assert!(log_path.exists());

        let content = std::fs::read_to_string(log_path.to_str().unwrap()).expect(&format!("read {} failed.", log_path.to_str().unwrap()));
        assert!(content.contains(DEBUG_INFO));        
    }
}