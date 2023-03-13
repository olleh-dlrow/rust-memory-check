/*
 * @Author: Shuwen Chen 
 * @Date: 2023-03-13 00:35:32 
 * @Last Modified by: Shuwen Chen
 * @Last Modified time: 2023-03-13 20:04:01
 */
use std::io::Write;

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
        .replace("{$PATH}", tmp_dir.as_path().join(LOG_FILENAME).to_str().unwrap())
        .replace("{$LEVEL}", &level.to_string().to_lowercase());

    writeln!(cfg_file, "{}", yaml_str)?;

    log4rs::init_file(cfg_abs_path.to_str().unwrap(), Default::default()).unwrap();

    Ok(())
}