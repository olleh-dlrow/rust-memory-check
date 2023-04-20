#![feature(rustc_private)]
#![feature(box_patterns)]
#![feature(allocator_api)]
#[macro_use]
extern crate lazy_static;

extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_index;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

pub mod core;
use crate::core::check;
use crate::core::{analysis, pfg::PointerFlowGraph, AnalysisOptions, CallerContext, CtxtSenCallId};
use termcolor::{Color};

use crate::core::GlobalBasicBlockId;
use std::collections::{HashMap, HashSet, VecDeque};

use rustc_hir::def_id::DefId;

use crate::core::{cfg::ControlFlowGraph, utils};
use crate::core::cfg;

pub fn analysis_then_check() -> Result<(), rustc_errors::ErrorGuaranteed> {
    rustc_driver::catch_fatal_errors(move || {

        // behaviour like the real rustc
        if std::env::var_os("MEMORY_CHECK_BE_RUSTC").is_some() {
            let rustc_args = get_rustc_args(true);
            // log::debug!("rustc args: {:?}", rustc_args);
            let (_, rustc_args) = utils::parse_args(&rustc_args);
            rustc_driver::init_rustc_env_logger();
            let mut callbacks = rustc_driver::TimePassesCallbacks::default();
            rustc_driver::RunCompiler::new(&rustc_args, &mut callbacks).run()
        } else {
            let rustc_args = get_rustc_args(false);
            // log::debug!("rustc args: {:?}", rustc_args);
            let (options, rustc_args) = utils::parse_args(&rustc_args);
            if utils::open_dbg(&options) {
                utils::init_log(log::Level::Debug).expect("init log failed");
            }
            let mut callbacks = MemoryCheckCallbacks { options };
            rustc_driver::RunCompiler::new(&rustc_args, &mut callbacks).run()
        }
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

            let mut called_infos = HashMap::<DefId, HashSet<GlobalBasicBlockId>>::new();

            // create control flow graphs
            tcx.hir().body_owners().for_each(|local_def_id| {
                let def_id = local_def_id.to_def_id();

                if let Some(cfg) = cfg::try_create_cfg(&self.options, tcx, def_id, true) {
                    assert!(!cfgs.contains_key(&def_id));
                    cfg::add_called_info(&self.options, &mut called_infos, &cfg);
                    cfgs.insert(def_id, cfg);
                }
            });

            if utils::has_dbg(&self.options, "defid") {
                let def_ids = cfgs.keys().collect::<Vec<_>>();
                log::debug!("def ids: {:#?}", def_ids);
            }

            // auto or manual detect entries
            let entry_def_ids = if utils::auto_detect_entries(&self.options) {
                check::output_level_text("info", "auto detect entries");
                utils::get_top_def_ids(&cfgs)
            } else {
                cfgs.keys()
                    .filter(|def_id| utils::has_entry(&self.options, **def_id))
                    .map(|def_id| *def_id)
                    .collect::<Vec<_>>()
            };

            // output entries 
            if !entry_def_ids.is_empty() {
                check::output_level_text("info", "analysis from entries:");
                for entry_def_id in entry_def_ids.iter() {
                    utils::print_with_color(" - ", Color::Blue).unwrap();
                    utils::println_with_color(&utils::parse_def_id(*entry_def_id).join("::"), Color::White).unwrap();
                }
            } else {
                check::output_level_text("warning", "without entry");
            }

            // collect check infos
            let mut check_infos = HashMap::new();

            for entry_def_id in entry_def_ids.iter() {
                log::debug!("entry def id: {:?}", entry_def_id);

                let ctxt = analysis::AnalysisContext {
                    options: self.options.clone(),
                    tcx,
                    cfgs: cfgs,
                    called_infos: called_infos,
                    pfg: PointerFlowGraph::new(),
                    cs_reachable_calls: HashSet::new(),
                    worklist: VecDeque::new(),
                };

                let ctxt = analysis::alias_analysis(
                    ctxt,
                    CtxtSenCallId::new(*entry_def_id, CallerContext::new(vec![])),
                );

                let check_info = check::check_memory_bug(&ctxt);

                cfgs = ctxt.cfgs;
                called_infos = ctxt.called_infos;

                check_infos.insert(*entry_def_id, check_info);
            }

            if utils::has_dbg(&self.options, "check-info") {
                log::debug!("check infos: {:#?}", check_infos);
            }
            let check_result = check::merge_check_info(&cfgs, &check_infos);
            if utils::has_dbg(&self.options, "check-result") {
                log::debug!("check result: {:#?}", check_result);
            }
            check::output_check_result(&check_result);
        });
        rustc_driver::Compilation::Continue
    }
}

fn get_rustc_args(is_rustc: bool) -> Vec<String> {
    let mut rustc_args = std::env::args().into_iter().collect::<Vec<String>>();

    // Get MIR code for all code related to the crate (including the dependencies and standard library)
    let always_encode_mir = "-Zalways_encode_mir";
    if !rustc_args.iter().any(|arg| arg == always_encode_mir) {
        rustc_args.push(always_encode_mir.to_owned());
    }

    // WARNING!!!
    // can't add this flag to all, because it will cause SIGSEGV: invalid memory reference
    // Add this to support analyzing no_std libraries
    if !is_rustc {
        rustc_args.push("-Clink-arg=-nostartfiles".to_owned());
    }

    // Disable unwind to simplify the CFG
    rustc_args.push("-Cpanic=abort".to_owned());

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

    #[test]
    fn test_entry_is_suffix_of() {
        let entry = vec!["b".to_string(), "c".to_string()];
        let def_id = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(utils::entry_is_suffix_of(&entry, &def_id), true);
    }
}
