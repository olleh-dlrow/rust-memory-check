use std::{io::{Write, BufReader}, collections::{HashMap, HashSet}};
use std::io::{BufRead};

use rustc_hir::def_id::DefId;
use rustc_span::Span;

use crate::core::AnalysisOptions;

use super::{cfg::ControlFlowGraph, GlobalBasicBlockId, BasicBlockId};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

pub const LOG4RS_CONFIG_YAML: &str = r#"
# appender: collect logs to console or file, could have multiple configs
# ref: https://zhuanlan.zhihu.com/p/104921298
appenders:
  # stdout:
  #   kind: console
  #   # https://docs.rs/log4rs/0.10.0/log4rs/encode/pattern/index.html
  #   encoder:
  #     pattern: "{d(%Y-%m-%d %H:%M:%S)} [{f}:{L}] [{h({l:<5})}]  {m}{n}"
  file:
    kind: file
    path: "{$PATH}"
    append: false
    encoder:
      pattern: "{d(%Y-%m-%d %H:%M:%S)} [{f}:{L}] [{h({l:<5})}]  {m}{n}"
# global config
root:
  level: {$LEVEL}
  appenders:
    # - stdout
    - file
"#;

pub const CONFIG_FILENAME: &str = "mc-log4rs-config.yaml";
pub const LOG_FILENAME: &str = "mc-log4rs-output.log";

/// A public function that initializes the log4rs logger with a given level
/// Returns Ok(()) if successful, or an I/O error if not
pub fn init_log(level: log::Level) -> Result<(), std::io::Error> {
    let tmp_dir = std::env::temp_dir();
    let cfg_abs_path = tmp_dir.as_path().join(CONFIG_FILENAME);

    let mut cfg_file = std::fs::File::create(cfg_abs_path.clone())?;

    let yaml_str = LOG4RS_CONFIG_YAML
        .replace(
            "{$PATH}",
            tmp_dir.as_path().join(LOG_FILENAME).to_str().unwrap(),
        )
        .replace("{$LEVEL}", &level.to_string().to_lowercase());

    writeln!(cfg_file, "{}", yaml_str)?;

    log4rs::init_file(cfg_abs_path.to_str().unwrap(), Default::default()).unwrap();

    Ok(())
}

pub fn has_dbg(opts: &AnalysisOptions, opt_name: &str) -> bool {
    opts.debug_opts.iter().any(|s| s == opt_name)
}

pub fn has_entry(opts: &AnalysisOptions, def_id: DefId) -> bool {
    opts.entries.iter().any(|entry| entry_is_suffix_of(&parse_entry(entry), &parse_def_id(def_id)))
}

pub fn auto_detect_entries(opts: &AnalysisOptions) -> bool {
    opts.entries.is_empty()
}

pub fn open_dbg(opts: &AnalysisOptions) -> bool {
    opts.open_dbg
}

pub fn parse_args(args: &[String]) -> (AnalysisOptions, Vec<String>) {
    let mut index_removed = vec![];
    let mut debug_opts = vec![];
    let mut entries = vec![];
    let mut open_dbg = false;


    let mut try_get_arg_value = |name: &str| {
        for (i, arg) in args.iter().enumerate() {
            if !arg.starts_with(name) {
                continue;
            }
            let suffix = &arg[name.len()..];
            if suffix.starts_with('=') {
                index_removed.push(i);
                return Some(suffix[1..].to_owned());
            }
        }
        return None;
    };

    if let Some(arg) = try_get_arg_value("--DBG") {
        debug_opts.extend(arg.split(',').map(|s| s.to_owned()));
    }

    if let Some(arg) = try_get_arg_value("--entries") {
        entries.extend(arg.split(',').map(|s| s.to_owned()));
    }

    if let Some(arg) = try_get_arg_value("--open-dbg") {
        open_dbg = arg == "1";
    }

    let new_args = args
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !index_removed.contains(i))
        .map(|(_, s)| s.to_owned())
        .collect::<Vec<String>>();
    (AnalysisOptions { debug_opts, entries, open_dbg }, new_args)
}

pub fn get_ty_from_place<'tcx>(
    tcx: rustc_middle::ty::TyCtxt<'tcx>,
    def_id: rustc_hir::def_id::DefId,
    place: &rustc_middle::mir::Place<'tcx>,
) -> rustc_middle::ty::Ty<'tcx> {
    let body = tcx.optimized_mir(def_id);
    let place_ty = place.ty(&body.local_decls, tcx);
    if place_ty.variant_index.is_some() {
        log::debug!(
            "unhandled PlaceTy::variant_index: {:?}",
            place_ty.variant_index
        );
    }
    place_ty.ty
}

pub fn can_call_arrive(cfgs: &HashMap<DefId, ControlFlowGraph>, visited: &mut HashSet<DefId>, from: DefId, to: DefId) -> bool {
    if from == to {
        return true;
    }

    if visited.contains(&from) {
        return false;
    }

    visited.insert(from);

    let from_cfg = cfgs.get(&from).unwrap();
    for (_, call_info) in from_cfg.call_infos.iter() {
        let next_id = call_info.callee_def_id;
        if cfgs.contains_key(&next_id) {
            if can_call_arrive(cfgs, visited, next_id, to) {
                return true;
            }
        }
    }

    return false;
}

pub fn can_basic_block_arrive(cfgs: &HashMap<DefId, ControlFlowGraph>, visited: &mut HashSet<GlobalBasicBlockId>, from: GlobalBasicBlockId, to: GlobalBasicBlockId) -> bool {
    if from == to {
        return true;
    }

    if visited.contains(&from) {
        return false;
    }

    if !can_call_arrive(cfgs, &mut HashSet::new(), from.def_id, to.def_id) {
        return false;
    }

    visited.insert(from);
    
    if from.def_id == to.def_id {
        return can_inner_basic_block_arrive(cfgs.get(&from.def_id).unwrap(), &mut HashSet::new(), from.bb_id, to.bb_id);
    } else {
        let from_cfg = cfgs.get(&from.def_id).unwrap();
        for (bb_id, call_info) in from_cfg.call_infos.iter() {
            // register call
            if cfgs.contains_key(&call_info.callee_def_id) {
                // can internal transit
                if can_inner_basic_block_arrive(from_cfg, &mut HashSet::new(), from.bb_id, *bb_id) {
                    // walk to caller site, then transfer to the begin of callee, check next
                    if can_basic_block_arrive(cfgs, visited, GlobalBasicBlockId::new(call_info.callee_def_id, BasicBlockId::from_usize(0)), to) {
                        return true;
                    }                                
                }
            }
        }

        return false;
    }
}

pub fn can_inner_basic_block_arrive(cfg: &ControlFlowGraph, visited: &mut HashSet<BasicBlockId>, from: BasicBlockId, to: BasicBlockId) -> bool {
    if from == to {
        return true;
    }

    if visited.contains(&from) {
        return false;
    }

    visited.insert(from);

    for successor in cfg.basic_block_infos.get(&from).unwrap().successors.iter() {
        if can_inner_basic_block_arrive(cfg, visited, *successor, to) {
            return true;
        }
    }

    return false;
}

// eg：src/main.rs:1:2: 3:4 (#0)
// parse：filename: src/main.rs, line_range: (1, 3), column_range: (2, 4)
pub fn parse_span(span: &Span) -> (String, (usize, usize), (usize, usize)) {
    let span_str = format!("{:?}", span);
    let mut span_str_iter = span_str.split("(#");
    let mut span_str_iter = span_str_iter.next().unwrap().split(':');

    let filename = span_str_iter.next().unwrap().to_string();
    
    let line_lo = span_str_iter.next().unwrap().parse::<usize>().unwrap();
    let column_lo = span_str_iter.next().unwrap().parse::<usize>().unwrap();
    let line_hi = span_str_iter.next().unwrap().trim().parse::<usize>().unwrap();
    let column_hi = span_str_iter.next().unwrap().trim().parse::<usize>().unwrap();

    let line_range = (line_lo, line_hi);
    let column_range = (column_lo, column_hi);
    (filename, line_range, column_range)
}

// DefId(0:4 ~ test02[fd64]::utils::foo)
pub fn parse_def_id(def_id: DefId) -> Vec<String> {
    let def_id_str = format!("{:?}", def_id);
    let mut def_id_str_iter = def_id_str.split(" ~ ");
    let _ = def_id_str_iter.next();
    let def_id_str_iter = def_id_str_iter.next().unwrap().split("::");
    
    def_id_str_iter.map(|s| {
        let mut s_iter = s.split('[');
        let s = s_iter.next().unwrap();

        let mut s_iter = s.split(')');
        let s = s_iter.next().unwrap();

        s.to_string()
    }).collect()    
}

pub fn parse_entry(entry: &str) -> Vec<String> {
    let entry_iter = entry.split("::");
    entry_iter.map(|s| s.to_string()).collect()
}

pub fn entry_is_suffix_of(entry: &Vec<String>, def_id: &Vec<String>) -> bool {
    if entry.len() > def_id.len() {
        return false;
    }

    for i in 0..entry.len() {
        if entry[entry.len() - i - 1] != def_id[def_id.len() - i - 1] {
            return false;
        }
    }

    return true;
}

// get nodes with indegree is 0, which means they are the entry of the call graph. Loop edges are not considered.
pub fn get_top_def_ids(cfgs: &HashMap<DefId, ControlFlowGraph>) -> Vec<DefId> {
    
    let mut in_degree = cfgs.keys().map(|k| (*k, 0)).collect::<HashMap<DefId, usize>>();

    for (def_id, cfg) in cfgs.iter() {
        for (_, call_info) in cfg.call_infos.iter() {
            if cfgs.contains_key(&call_info.callee_def_id) && call_info.callee_def_id != *def_id {
                in_degree.entry(call_info.callee_def_id).and_modify(|e| *e += 1);
            }
        }
    }

    in_degree.iter().filter(|(_, v)| **v == 0).map(|(k, _)| *k).collect()
}

pub fn print_with_color(text: &str, color: Color) -> Result<(), std::io::Error> {
    let mut stdout = StandardStream::stdout(ColorChoice::Always);
    stdout.set_color(ColorSpec::new().set_fg(Some(color)))?;
    write!(&mut stdout, "{}", text)?;
    stdout.reset()?;
    Ok(())
}

pub fn println_with_color(text: &str, color: Color) -> Result<(), std::io::Error> {
    let mut stdout = StandardStream::stdout(ColorChoice::Always);
    stdout.set_color(ColorSpec::new().set_fg(Some(color)))?;
    writeln!(&mut stdout, "{}", text)?;
    stdout.reset()?;
    Ok(())
}


pub fn get_lines_in_file(file_path: &str, line_range: (usize, usize)) -> Vec<String> {
    let mut lines = Vec::new();

    let file = std::fs::File::open(file_path).unwrap();
    let reader = BufReader::new(file);

    for (i, line) in reader.lines().enumerate() {
        let i = i + 1;
        if i >= line_range.0 && i <= line_range.1 {
            lines.push(line.unwrap());
        }
    }

    lines
}