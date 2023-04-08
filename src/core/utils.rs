/*
 * @Author: Shuwen Chen
 * @Date: 2023-03-13 00:35:32
 * @Last Modified by: Shuwen Chen
 * @Last Modified time: 2023-03-13 20:04:01
 */
use std::{io::Write};

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

#[derive(Clone, Debug)]
pub struct AnalysisOptions {
    pub debug_opts: Vec<String>,
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
