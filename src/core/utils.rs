use std::io::Write;

use crate::core::AnalysisOptions;

use super::LocalId;

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

pub fn parse_args(args: &[String]) -> (AnalysisOptions, Vec<String>) {
    let mut index_removed = vec![];
    let mut debug_opts = vec![];
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

    let new_args = args
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !index_removed.contains(i))
        .map(|(_, s)| s.to_owned())
        .collect::<Vec<String>>();
    (AnalysisOptions { debug_opts }, new_args)
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
