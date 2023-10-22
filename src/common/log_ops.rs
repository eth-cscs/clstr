use std::str::FromStr;

use log::LevelFilter;
use log4rs::{
    append::console::ConsoleAppender,
    config::{Appender, Logger, Root},
    encode::pattern::PatternEncoder,
    Config,
};

// Code base log4rs configuration to avoid having a separate file for this to keep portability
pub fn configure(log_level: String) {
    let stdout = ConsoleAppender::builder()
        .encoder(Box::new(PatternEncoder::new(
            "{d(%Y-%m-%d %H:%M:%S)} | {h({l}):5.5} | {f}:{L} — {m}{n}",
        )))
        .build();

    let config = Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .logger(
            Logger::builder()
                .appender("stdout")
                .build("app::backend", LevelFilter::Info),
        )
        .build(
            Root::builder()
                .appender("stdout")
                .build(LevelFilter::from_str(&log_level).unwrap_or(LevelFilter::Error)),
        )
        .unwrap();

    let _handle = log4rs::init_config(config).unwrap();

    // use handle to change logger configuration at runtime
}
