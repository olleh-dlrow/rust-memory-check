#![feature(rustc_private)]
#![feature(box_patterns)]
#![feature(allocator_api)]

extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

pub mod core;

use crate::core::{AnalysisOptions, analysis, pfg::PointerFlowGraph, CtxtSenCallId, CallerContext};

use std::collections::{HashMap, HashSet, VecDeque};

use rustc_hir::def_id::DefId;

use crate::core::{cfg::ControlFlowGraph, utils};

pub fn analysis_then_check() -> Result<(), rustc_errors::ErrorGuaranteed> {
    rustc_driver::catch_fatal_errors(move || {
        let rustc_args = get_rustc_args();
        let (options, rustc_args) = utils::parse_args(&rustc_args);

        let mut callbacks = MemoryCheckCallbacks { options };
        rustc_driver::RunCompiler::new(&rustc_args, &mut callbacks).run()
    })
    .and_then(|result| result)
}

struct MemoryCheckCallbacks {
    options: AnalysisOptions,
}

impl rustc_driver::Callbacks for MemoryCheckCallbacks {
    fn after_analysis<'tcx>(
        &mut self,
        compiler: &rustc_interface::interface::Compiler,
        queries: &'tcx rustc_interface::Queries<'tcx>,
    ) -> rustc_driver::Compilation {
        compiler.session().abort_if_errors();

        queries.global_ctxt().unwrap().peek_mut().enter(|tcx| {
            let mut cfgs: HashMap<DefId, ControlFlowGraph> = HashMap::new();
            let mut entry_def_id: Option<DefId> = None;

            tcx.hir().body_owners().for_each(|local_def_id| {
                // analysis_then_check_body(tcx, local_def_id.to_def_id())
                let def_id = local_def_id.to_def_id();
                let def_name = format!("{:?}", def_id);
                if def_name.contains("main") {
                    entry_def_id = Some(def_id);
                }

                if let Some(cfg) = try_get_cfg(&self.options, tcx, def_id) {
                    assert!(!cfgs.contains_key(&def_id));
                    cfgs.insert(def_id, cfg);
                }
            });

            if let Some(entry_def_id) = entry_def_id {
                log::debug!("entry def id: {:?}", entry_def_id);

                let ctxt = analysis::AnalysisContext {
                    options: self.options.clone(),
                    tcx,
                    cfgs,
                    pfg: PointerFlowGraph::new(),
                    cs_reachable_calls: HashSet::new(),
                    worklist: VecDeque::new(),
                };

                // TODO: analysis from entry call
                let ctxt = analysis::alias_analysis(ctxt, CtxtSenCallId::new(entry_def_id, CallerContext::new(vec![])));
            }
        });
        rustc_driver::Compilation::Continue
    }
}

fn try_get_cfg<'tcx>(opts: &AnalysisOptions, tcx: rustc_middle::ty::TyCtxt<'tcx>, def_id: DefId) -> Option<ControlFlowGraph<'tcx>> {
    if let Some(other) = tcx.hir().body_const_context(def_id.expect_local()) {
        log::debug!("ignore const context of def id {:?}: {:?}", def_id, other);
        return None;
    }

    if tcx.is_mir_available(def_id) {
        let cfg = ControlFlowGraph::new(opts, tcx, def_id);
        if utils::has_dbg(&opts, "cfg") {
            log::debug!("control flow graph of def id {:?}: {:#?}", def_id, cfg);
        }
        Some(cfg)
    } else {
        log::debug!("MIR is unavailable for def id {:?}", def_id);
        None
    }
}

// fn analysis_then_check_body(opts: &AnalysisOptions, tcx: rustc_middle::ty::TyCtxt, def_id: rustc_hir::def_id::DefId) {
//     if let Some(other) = tcx.hir().body_const_context(def_id.expect_local()) {
//         log::debug!("ignore const context of def id {:?}: {:?}", def_id, other);
//         return;
//     }

//     if tcx.is_mir_available(def_id) {
//         let mut cfg = ControlFlowGraph::new(opts, tcx, def_id);
//         log::debug!("control flow graph of def id {:?}: {:#?}", def_id, cfg);
//         crate::core::analysis::alias_analysis(&mut cfg);
//         let _report = crate::core::check::check_then_report(&mut cfg);
//     } else {
//         log::debug!("MIR is unavailable for def id {:?}", def_id);
//     }
// }

fn get_rustc_args() -> Vec<String> {
    let mut rustc_args = std::env::args().into_iter().collect::<Vec<String>>();

    // Get MIR code for all code related to the crate (including the dependencies and standard library)
    let always_encode_mir = "-Zalways_encode_mir";
    if !rustc_args.iter().any(|arg| arg == always_encode_mir) {
        rustc_args.push(always_encode_mir.to_owned());
    }

    // Add this to support analyzing no_std libraries
    // rustc_args.push("-Clink-arg=-nostartfiles".to_owned());

    // add sysroot
    if let Some(sysroot) = get_compile_time_sysroot() {
        let sysroot_flag = "--sysroot";
        if !rustc_args.iter().any(|arg| arg == sysroot_flag) {
            rustc_args.extend([sysroot_flag.to_owned(), sysroot]);
        }
    }

    rustc_args
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
            .expect("To build without rustup, set the `RUST_SYSROOT` env var at build time")
            .to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use crate::core::utils;
    #[test]
    fn test_log() {
        const DEBUG_INFO: &str = "TEST LOG";

        assert!(utils::init_log(log::Level::Debug).is_ok());
        log::debug!("{}", DEBUG_INFO);

        let tmp_dir = std::env::temp_dir();
        let log_path = tmp_dir.as_path().join(utils::LOG_FILENAME);
        assert!(tmp_dir.as_path().join(utils::CONFIG_FILENAME).exists());
        assert!(log_path.exists());

        let content = std::fs::read_to_string(log_path.to_str().unwrap())
            .expect(&format!("read {} failed.", log_path.to_str().unwrap()));
        assert!(content.contains(DEBUG_INFO));
    }

    #[test]
    fn test_arg_parse() {
        let args = vec![
            "mc".to_owned(),
            "--sysroot".to_owned(),
            "/home/xxx/.rustup/toolchains/stable-x86_64-unknown-linux-gnu".to_owned(),
            "--DBG=cfg,body".to_owned(),
        ];

        let (options, _) = utils::parse_args(&args);
        assert_eq!(options.debug_opts, vec!["cfg", "body"]);
    }
}
