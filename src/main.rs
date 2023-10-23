mod cli;
mod common;

use mesa::shasta;
use std::path::PathBuf;

use directories::ProjectDirs;

use shasta::authentication;

use crate::common::log_ops;

// DHAT (profiling)
// #[cfg(feature = "dhat-heap")]
// #[global_allocator]
// static ALOC: dhat::Alloc = dhat::Alloc;

#[tokio::main]
async fn main() -> core::result::Result<(), Box<dyn std::error::Error>> {
    // DHAT (profiling)
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    // XDG Base Directory Specification
    let project_dirs = ProjectDirs::from(
        "local", /*qualifier*/
        "cscs",  /*organization*/
        "manta", /*application*/
    );

    let mut path_to_manta_configuration_file = PathBuf::from(project_dirs.unwrap().config_dir());

    path_to_manta_configuration_file.push("config.toml"); // ~/.config/manta/config is the file

    log::info!(
        "Reading manta configuration from {}",
        &path_to_manta_configuration_file.to_string_lossy()
    );

    let settings = common::config_ops::get_configuration();

    let shasta_base_url = settings.get_string("shasta_base_url").unwrap();
    let keycloak_base_url = settings.get_string("keycloak_base_url").unwrap();
    let log_level = settings.get_string("log").unwrap_or("error".to_string());

    // Init logger
    // env_logger::init();
    // log4rs::init_file("log4rs.yml", Default::default()).unwrap(); // log4rs file configuration
    log_ops::configure(log_level); // log4rs programatically configuration

    if let Ok(socks_proxy) = settings.get_string("socks5_proxy") {
        std::env::set_var("SOCKS5", socks_proxy);
    }

    let settings_hsm_group_opt = settings.get_string("hsm_group").ok();

    let shasta_root_cert = common::config_ops::get_csm_root_cert_content();

    let shasta_token =
        authentication::get_api_token(&shasta_base_url, &shasta_root_cert, &keycloak_base_url)
            .await?;

    // Process input params
    let matches = crate::cli::build::build_cli(settings_hsm_group_opt.as_ref()).get_matches();
    let cli_result = crate::cli::process::process_cli(
        matches,
        &shasta_token,
        &shasta_base_url,
        &shasta_root_cert,
        settings_hsm_group_opt.as_ref(),
    )
    .await;

    match cli_result {
        Ok(_) => Ok(()),
        Err(e) => panic!("{}", e),
    }
}
